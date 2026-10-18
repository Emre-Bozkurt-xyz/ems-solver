import asyncio
import json
import logging
import time
from typing import AsyncIterator

from fastapi import FastAPI, Request
from fastapi.responses import JSONResponse, StreamingResponse
from pydantic import ValidationError

from .auth import RateLimiter, RequestVerifier
from .config import Settings
from .content import to_claude_content
from .errors import ApiError
from .schemas import ImagePart, SolveRequest
from .solver import ClaudeSolver, Completed, SolveEvent, Solver, TextDelta

log = logging.getLogger("ems_solver")

_KEEPALIVE = object()
_INTERNAL_ERROR = ApiError(500, "internal_error", "unexpected server error", retryable=True)


async def _read_body(request: Request, limit: int) -> bytes:
    declared = request.headers.get("content-length", "")
    if declared.isdigit() and int(declared) > limit:
        raise ApiError(413, "payload_too_large", f"request body exceeds {limit} bytes")
    chunks, size = [], 0
    async for chunk in request.stream():
        size += len(chunk)
        if size > limit:
            raise ApiError(413, "payload_too_large", f"request body exceeds {limit} bytes")
        chunks.append(chunk)
    return b"".join(chunks)


def _validation_message(exc: ValidationError) -> str:
    first = exc.errors()[0]
    location = ".".join(str(p) for p in first["loc"]) or "body"
    return f"{location}: {first['msg']}"


def _sse(event: str, data: dict) -> str:
    return f"event: {event}\ndata: {json.dumps(data, ensure_ascii=False)}\n\n"


def _completed_payload(done: Completed) -> dict:
    return {
        "model": done.model,
        "stop_reason": done.stop_reason,
        "usage": {"input_tokens": done.usage.input_tokens, "output_tokens": done.usage.output_tokens},
    }


async def _with_keepalive(events: AsyncIterator[SolveEvent], interval: float) -> AsyncIterator[object]:
    """Re-yield `events`, yielding _KEEPALIVE whenever nothing arrives for `interval` seconds."""
    queue: asyncio.Queue = asyncio.Queue()
    finished = object()

    async def pump() -> None:
        try:
            async for item in events:
                await queue.put(item)
        except Exception as exc:
            await queue.put(exc)
        else:
            await queue.put(finished)

    task = asyncio.create_task(pump())
    try:
        while True:
            try:
                item = await asyncio.wait_for(queue.get(), interval)
            except asyncio.TimeoutError:
                yield _KEEPALIVE
                continue
            if item is finished:
                return
            if isinstance(item, Exception):
                raise item
            yield item
    finally:
        task.cancel()


def create_app(settings: Settings, solver: Solver | None = None) -> FastAPI:
    verifier = RequestVerifier(
        settings.client_secrets, settings.auth_max_skew_seconds, settings.auth_nonce_ttl_seconds
    )
    limiter = RateLimiter(settings.rate_limit_per_minute)
    solver = solver or ClaudeSolver(settings)

    app = FastAPI(title="ems-solver", docs_url=None, redoc_url=None, openapi_url=None)

    @app.exception_handler(ApiError)
    async def handle_api_error(_: Request, exc: ApiError) -> JSONResponse:
        return JSONResponse({"ok": False, "error": exc.to_dict()}, status_code=exc.status)

    @app.exception_handler(Exception)
    async def handle_unexpected(_: Request, exc: Exception) -> JSONResponse:
        log.exception("unexpected error", exc_info=exc)
        return JSONResponse({"ok": False, "error": _INTERNAL_ERROR.to_dict()}, status_code=500)

    async def accept(request: Request) -> tuple[str, SolveRequest, list[dict]]:
        body = await _read_body(request, settings.max_body_bytes)
        client_id = verifier.verify(request.method, request.url.path, request.headers, body)
        limiter.check(client_id)
        try:
            solve_request = SolveRequest.model_validate_json(body)
        except ValidationError as exc:
            raise ApiError(400, "invalid_request", _validation_message(exc)) from None
        return client_id, solve_request, to_claude_content(solve_request.parts, settings)

    def log_outcome(client_id: str, req: SolveRequest, started: float, outcome: str, done: Completed | None = None):
        images = sum(isinstance(p, ImagePart) for p in req.parts)
        tokens = f" in={done.usage.input_tokens} out={done.usage.output_tokens}" if done else ""
        log.info(
            "solve client=%s mode=%s parts=%d images=%d outcome=%s %.1fs%s",
            client_id, req.mode, len(req.parts), images, outcome, time.monotonic() - started, tokens,
        )

    @app.get("/healthz")
    async def healthz():
        return {"ok": True}

    @app.post("/v1/ping")
    async def ping(request: Request):
        # Lets clients check their credentials without calling the model.
        body = await _read_body(request, settings.max_body_bytes)
        client_id = verifier.verify(request.method, request.url.path, request.headers, body)
        return {"ok": True, "client_id": client_id}

    @app.post("/v1/solve")
    async def solve(request: Request):
        client_id, req, blocks = await accept(request)
        started = time.monotonic()
        try:
            done = None
            async for event in solver.stream(req.mode, blocks):
                if isinstance(event, Completed):
                    done = event
        except ApiError as exc:
            log_outcome(client_id, req, started, exc.code)
            raise
        log_outcome(client_id, req, started, "ok", done)
        return {"ok": True, "result": done.text, **_completed_payload(done)}

    @app.post("/v1/solve/stream")
    async def solve_stream(request: Request):
        # Auth and validation errors are returned as ordinary JSON responses;
        # once the stream has started, failures arrive as an `error` event.
        client_id, req, blocks = await accept(request)

        async def events() -> AsyncIterator[str]:
            started = time.monotonic()
            try:
                async for item in _with_keepalive(solver.stream(req.mode, blocks), settings.sse_keepalive_seconds):
                    if item is _KEEPALIVE:
                        yield ": keepalive\n\n"
                    elif isinstance(item, TextDelta):
                        yield _sse("delta", {"text": item.text})
                    elif isinstance(item, Completed):
                        log_outcome(client_id, req, started, "ok", item)
                        yield _sse("done", {"result": item.text, **_completed_payload(item)})
            except ApiError as exc:
                log_outcome(client_id, req, started, exc.code)
                yield _sse("error", exc.to_dict())
            except Exception:
                log.exception("unexpected error while streaming")
                yield _sse("error", _INTERNAL_ERROR.to_dict())

        return StreamingResponse(
            events(),
            media_type="text/event-stream",
            headers={"Cache-Control": "no-cache", "X-Accel-Buffering": "no"},
        )

    return app
