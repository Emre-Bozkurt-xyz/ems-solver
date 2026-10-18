import base64

import pytest

from conftest import PNG_B64, FakeSolver, parse_sse, post_signed, text_request
from ems_solver.app import create_app
from ems_solver.errors import ApiError
from conftest import make_settings
from fastapi.testclient import TestClient


def image_part(data=PNG_B64, media_type="image/png"):
    return {"type": "image", "media_type": media_type, "data": data}


def test_healthz(client):
    assert client.get("/healthz").json() == {"ok": True}


def test_solve_returns_result(client, solver):
    response = post_signed(client, "/v1/solve", text_request("2+2?", mode="hint"))
    assert response.status_code == 200
    assert response.json() == {
        "ok": True,
        "result": "Hello world",
        "model": "fake-model",
        "stop_reason": "end_turn",
        "usage": {"input_tokens": 10, "output_tokens": 2},
    }
    mode, blocks = solver.calls[0]
    assert mode == "hint"
    assert blocks == [{"type": "text", "text": "2+2?"}]


def test_mixed_parts_keep_order(client, solver):
    payload = {"parts": [{"type": "text", "text": "context"}, image_part(), {"type": "text", "text": "question"}]}
    assert post_signed(client, "/v1/solve", payload).status_code == 200
    _, blocks = solver.calls[0]
    assert [b["type"] for b in blocks] == ["text", "image", "text"]
    assert blocks[1]["source"] == {"type": "base64", "media_type": "image/png", "data": PNG_B64}


@pytest.mark.parametrize(
    "payload, fragment",
    [
        ({"parts": []}, "parts"),
        ({"parts": [{"type": "audio", "data": "x"}]}, "parts.0"),
        ({"mode": "essay", "parts": [{"type": "text", "text": "x"}]}, "mode"),
        ({"parts": [{"type": "text", "text": "x"}], "extra": 1}, "extra"),
        ({"parts": [{"type": "text", "text": "   "}]}, "blank"),
        ({"parts": [image_part(data="not base64!!")]}, "base64"),
        ({"parts": [image_part(media_type="image/jpeg")]}, "does not match"),
    ],
)
def test_invalid_requests(client, solver, payload, fragment):
    response = post_signed(client, "/v1/solve", payload)
    assert response.status_code == 400
    error = response.json()["error"]
    assert error["code"] == "invalid_request"
    assert fragment in error["message"]
    assert not solver.calls


def test_invalid_json(client):
    response = post_signed(client, "/v1/solve", b"{nope")
    assert response.status_code == 400


def test_webp_and_gif_signatures(client):
    webp = base64.b64encode(b"RIFF\x00\x00\x00\x00WEBPVP8 ").decode()
    gif = base64.b64encode(b"GIF89a" + b"\x00" * 8).decode()
    payload = {"parts": [image_part(webp, "image/webp"), image_part(gif, "image/gif")]}
    assert post_signed(client, "/v1/solve", payload).status_code == 200


def test_limits(make_client):
    client = make_client(max_parts=2, max_images=1, max_text_chars=10, max_image_bytes=16, max_body_bytes=4096)

    def code(payload):
        response = post_signed(client, "/v1/solve", payload)
        return response.status_code, response.json()["error"]["code"]

    assert code({"parts": [{"type": "text", "text": "a"}] * 3}) == (400, "invalid_request")
    assert code({"parts": [image_part(), image_part()]}) == (400, "invalid_request")
    assert code(text_request("x" * 11)) == (413, "payload_too_large")
    # PNG_BYTES is 40 bytes, over the 16-byte image limit.
    assert code({"parts": [image_part()]}) == (413, "payload_too_large")
    assert code(text_request("x" * 5000)) == (413, "payload_too_large")


def test_upstream_error_is_retryable(make_client, solver):
    solver.error = ApiError(502, "upstream_error", "Claude API returned 500", retryable=True)
    solver.deltas = ()
    response = post_signed(make_client(), "/v1/solve", text_request())
    assert response.status_code == 502
    assert response.json()["error"] == {
        "code": "upstream_error",
        "message": "Claude API returned 500",
        "retryable": True,
    }


def test_unexpected_exception_is_structured():
    class Broken:
        async def stream(self, mode, blocks):
            raise RuntimeError("boom")
            yield

    client = TestClient(create_app(make_settings(), Broken()), raise_server_exceptions=False)
    response = post_signed(client, "/v1/solve", text_request())
    assert response.status_code == 500
    assert response.json()["error"]["code"] == "internal_error"
    assert response.json()["error"]["retryable"] is True


def test_stream_success(client):
    response = post_signed(client, "/v1/solve/stream", text_request())
    assert response.status_code == 200
    assert response.headers["content-type"].startswith("text/event-stream")
    assert parse_sse(response.text) == [
        ("delta", {"text": "Hello"}),
        ("delta", {"text": " world"}),
        (
            "done",
            {
                "result": "Hello world",
                "model": "fake-model",
                "stop_reason": "end_turn",
                "usage": {"input_tokens": 10, "output_tokens": 2},
            },
        ),
    ]


def test_stream_error_after_partial_output(client, solver):
    solver.error = ApiError(422, "refused", "Claude declined this request", retryable=True)
    events = parse_sse(post_signed(client, "/v1/solve/stream", text_request()).text)
    assert events[-1] == ("error", {"code": "refused", "message": "Claude declined this request", "retryable": True})
    assert [name for name, _ in events] == ["delta", "delta", "error"]


def test_stream_auth_errors_are_plain_json(client):
    response = post_signed(client, "/v1/solve/stream", text_request(), secret="wrong")
    assert response.status_code == 401
    assert response.json()["error"]["code"] == "unauthorized"


def test_stream_sends_keepalives_while_waiting():
    slow = FakeSolver(delay=0.35)
    client = TestClient(create_app(make_settings(sse_keepalive_seconds=0.1), slow))
    raw = post_signed(client, "/v1/solve/stream", text_request()).text
    assert raw.count(": keepalive") >= 2
    assert parse_sse(raw)[-1][0] == "done"
