"""ClaudeSolver against a stand-in for the Anthropic client."""

import asyncio
from types import SimpleNamespace

import anthropic
import httpx2
import pytest

from conftest import make_settings
from ems_solver.errors import ApiError
from ems_solver.prompts import SYSTEM_PROMPT
from ems_solver.solver import FALLBACK_BETA, ClaudeSolver, Completed, TextDelta


class FakeStream:
    def __init__(self, texts, message):
        self._texts = texts
        self._message = message

    async def __aenter__(self):
        return self

    async def __aexit__(self, *exc):
        return False

    def __aiter__(self):
        return self._events()

    async def _events(self):
        yield SimpleNamespace(type="message_start")
        for text in self._texts:
            yield SimpleNamespace(type="text", text=text)

    async def get_final_message(self):
        return self._message


class FakeClient:
    def __init__(self, texts=(), message=None, error=None):
        self.kwargs = None
        self._texts, self._message, self._error = texts, message, error
        self.beta = SimpleNamespace(messages=SimpleNamespace(stream=self._stream))

    def _stream(self, **kwargs):
        self.kwargs = kwargs
        if self._error:
            raise self._error
        return FakeStream(self._texts, self._message)


def message(texts, stop_reason="end_turn", stop_details=None):
    return SimpleNamespace(
        content=[SimpleNamespace(type="thinking", thinking="")] + [SimpleNamespace(type="text", text=t) for t in texts],
        model="claude-sonnet-5-5",
        stop_reason=stop_reason,
        stop_details=stop_details,
        usage=SimpleNamespace(input_tokens=100, output_tokens=5),
    )


def collect(solver, mode="solution", blocks=None):
    async def run():
        return [e async for e in solver.stream(mode, blocks or [{"type": "text", "text": "2+2?"}])]

    return asyncio.run(run())


def test_request_shape_and_events():
    fake = FakeClient(texts=["4", "\n"], message=message(["4\n"]))
    events = collect(ClaudeSolver(make_settings(anthropic_effort="high"), fake), mode="hint")

    assert events[:2] == [TextDelta("4"), TextDelta("\n")]
    assert events[2] == Completed("4", "claude-sonnet-5-5", "end_turn", events[2].usage)

    kwargs = fake.kwargs
    assert kwargs["model"] == "claude-sonnet-5-5"
    assert kwargs["system"] == SYSTEM_PROMPT
    assert kwargs["output_config"] == {"effort": "high"}
    assert kwargs["betas"] == [FALLBACK_BETA]
    assert kwargs["fallbacks"] == "default"
    content = kwargs["messages"][0]["content"]
    assert content[0]["text"].startswith("Give only a hint")
    assert content[1] == {"type": "text", "text": "2+2?"}


def test_refusal_raises_retryable_error():
    fake = FakeClient(message=message([], stop_reason="refusal", stop_details=SimpleNamespace(category="cyber")))
    with pytest.raises(ApiError) as exc:
        collect(ClaudeSolver(make_settings(), fake))
    assert (exc.value.status, exc.value.code, exc.value.retryable) == (422, "refused", True)
    assert "cyber" in exc.value.message


def _request():
    return httpx2.Request("POST", "https://api.anthropic.com/v1/messages")


@pytest.mark.parametrize(
    "error, status, code",
    [
        (anthropic.APIConnectionError(request=_request()), 502, "upstream_unavailable"),
        (
            anthropic.RateLimitError("slow down", response=httpx2.Response(429, request=_request()), body=None),
            503,
            "upstream_rate_limited",
        ),
        (
            anthropic.InternalServerError("oops", response=httpx2.Response(500, request=_request()), body=None),
            502,
            "upstream_error",
        ),
    ],
)
def test_upstream_errors_are_mapped(error, status, code):
    with pytest.raises(ApiError) as exc:
        collect(ClaudeSolver(make_settings(), FakeClient(error=error)))
    assert (exc.value.status, exc.value.code, exc.value.retryable) == (status, code, True)
