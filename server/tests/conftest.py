import base64
import json
import time
import uuid

import pytest
from fastapi.testclient import TestClient

from ems_solver.app import create_app
from ems_solver.auth import HEADER_CLIENT_ID, HEADER_NONCE, HEADER_SIGNATURE, HEADER_TIMESTAMP, sign
from ems_solver.config import Settings
from ems_solver.errors import ApiError
from ems_solver.solver import Completed, TextDelta, Usage

CLIENT_ID = "tester"
SECRET = "s3cret"

PNG_BYTES = b"\x89PNG\r\n\x1a\n" + b"\x00" * 32
PNG_B64 = base64.b64encode(PNG_BYTES).decode()


class FakeSolver:
    """Records calls; streams `deltas`, then completes or raises `error`."""

    def __init__(self, deltas=("Hello", " world"), error: ApiError | None = None, delay: float = 0.0):
        self.deltas = deltas
        self.error = error
        self.delay = delay
        self.calls: list[tuple[str, list[dict]]] = []

    async def stream(self, mode, problem_blocks):
        import asyncio

        self.calls.append((mode, problem_blocks))
        if self.delay:
            await asyncio.sleep(self.delay)
        for text in self.deltas:
            yield TextDelta(text)
        if self.error:
            raise self.error
        yield Completed("".join(self.deltas), "fake-model", "end_turn", Usage(10, 2))


def make_settings(**overrides) -> Settings:
    values = dict(anthropic_api_key="test-key", client_secrets={CLIENT_ID: SECRET})
    values.update(overrides)
    return Settings(**values)


@pytest.fixture
def solver():
    return FakeSolver()


@pytest.fixture
def make_client(solver):
    def factory(**overrides) -> TestClient:
        return TestClient(create_app(make_settings(**overrides), solver), raise_server_exceptions=False)

    return factory


@pytest.fixture
def client(make_client):
    return make_client()


def signed_headers(body: bytes, path: str, *, client_id=CLIENT_ID, secret=SECRET, timestamp=None, nonce=None):
    timestamp = str(int(time.time()) if timestamp is None else timestamp)
    nonce = nonce or uuid.uuid4().hex
    return {
        "content-type": "application/json",
        HEADER_CLIENT_ID: client_id,
        HEADER_TIMESTAMP: timestamp,
        HEADER_NONCE: nonce,
        HEADER_SIGNATURE: sign(secret, "POST", path, client_id, timestamp, nonce, body),
    }


def post_signed(client: TestClient, path: str, payload, **sign_kwargs):
    body = payload if isinstance(payload, bytes) else json.dumps(payload).encode()
    return client.post(path, content=body, headers=signed_headers(body, path, **sign_kwargs))


def text_request(text="What is 2+2?", mode="solution"):
    return {"mode": mode, "parts": [{"type": "text", "text": text}]}


def parse_sse(raw: str) -> list[tuple[str, dict]]:
    events = []
    for frame in raw.split("\n\n"):
        lines = [line for line in frame.splitlines() if line and not line.startswith(":")]
        if not lines:
            continue
        fields = dict(line.split(": ", 1) for line in lines)
        events.append((fields["event"], json.loads(fields["data"])))
    return events
