import json
import time

import pytest

from conftest import CLIENT_ID, SECRET, post_signed, signed_headers, text_request
from ems_solver.auth import RateLimiter, RequestVerifier, canonical_string, sign
from ems_solver.errors import ApiError

PATH = "/v1/solve"


def test_canonical_string_format():
    body = b'{"a":1}'
    assert canonical_string("post", "/v1/solve", "c", "123", "n" * 16, body) == (
        b"EMS1\nPOST\n/v1/solve\nc\n123\n" + b"n" * 16 + b"\n"
        b"015abd7f5cc57a2dd94b7590f04ad8084273905ee33ec5cebeae62276a97f862"
    )


def test_valid_request_is_accepted(client):
    assert post_signed(client, PATH, text_request()).status_code == 200


def test_ping_checks_credentials_without_solving(client, solver):
    ok = post_signed(client, "/v1/ping", b"{}")
    assert ok.json() == {"ok": True, "client_id": CLIENT_ID}
    assert post_signed(client, "/v1/ping", b"{}", secret="wrong").status_code == 401
    assert not solver.calls


def test_missing_headers(client):
    response = client.post(PATH, json=text_request())
    assert response.status_code == 401
    assert response.json()["error"]["code"] == "unauthorized"


@pytest.mark.parametrize(
    "sign_kwargs",
    [
        {"client_id": "stranger"},
        {"secret": "wrong"},
    ],
)
def test_bad_credentials_look_identical(client, sign_kwargs):
    response = post_signed(client, PATH, text_request(), **sign_kwargs)
    assert response.status_code == 401
    assert response.json()["error"]["message"] == "invalid credentials"


def test_tampered_body_is_rejected(client):
    body = json.dumps(text_request("original")).encode()
    headers = signed_headers(body, PATH)
    tampered = json.dumps(text_request("tampered")).encode()
    assert client.post(PATH, content=tampered, headers=headers).status_code == 401


def test_signature_is_bound_to_path(client):
    body = json.dumps(text_request()).encode()
    headers = signed_headers(body, "/v1/solve/stream")
    assert client.post(PATH, content=body, headers=headers).status_code == 401


def test_stale_timestamp(client):
    response = post_signed(client, PATH, text_request(), timestamp=int(time.time()) - 600)
    assert response.status_code == 401
    assert "clock" in response.json()["error"]["message"]


def test_replay_is_rejected(client):
    body = json.dumps(text_request()).encode()
    headers = signed_headers(body, PATH)
    assert client.post(PATH, content=body, headers=headers).status_code == 200
    replay = client.post(PATH, content=body, headers=headers)
    assert replay.status_code == 401
    assert replay.json()["error"]["message"] == "replayed request"


def test_malformed_nonce(client):
    assert post_signed(client, PATH, text_request(), nonce="short").status_code == 401


def test_nonces_expire():
    now = [1000.0]
    verifier = RequestVerifier({CLIENT_ID: SECRET}, max_skew_seconds=10_000, nonce_ttl_seconds=60, clock=lambda: now[0])
    nonce = "a" * 32

    def headers():
        return {
            "x-ems-client-id": CLIENT_ID,
            "x-ems-timestamp": "1000",
            "x-ems-nonce": nonce,
            "x-ems-signature": sign(SECRET, "POST", PATH, CLIENT_ID, "1000", nonce, b""),
        }

    verifier.verify("POST", PATH, headers(), b"")
    with pytest.raises(ApiError):
        verifier.verify("POST", PATH, headers(), b"")
    now[0] += 61
    verifier.verify("POST", PATH, headers(), b"")


def test_rate_limit_window():
    now = [0.0]
    limiter = RateLimiter(2, clock=lambda: now[0])
    limiter.check("a")
    limiter.check("a")
    limiter.check("b")
    with pytest.raises(ApiError) as exc:
        limiter.check("a")
    assert exc.value.status == 429
    now[0] = 60
    limiter.check("a")


def test_rate_limit_applies_after_auth(make_client):
    client = make_client(rate_limit_per_minute=1)
    assert post_signed(client, PATH, text_request()).status_code == 200
    assert post_signed(client, PATH, text_request()).status_code == 429
    # Unauthenticated requests never consume the budget, but are still refused.
    assert post_signed(client, PATH, text_request(), secret="wrong").status_code == 401
