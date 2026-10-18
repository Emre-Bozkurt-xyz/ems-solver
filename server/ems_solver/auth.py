"""HMAC request signing.

The client signs a canonical string covering the method, path, its id, a
timestamp, a single-use nonce and the SHA-256 of the body:

    EMS1\\n{METHOD}\\n{path}\\n{client_id}\\n{timestamp}\\n{nonce}\\n{sha256_hex(body)}

and sends hex(HMAC-SHA256(secret, canonical)) along with the inputs in headers.
"""

import hashlib
import hmac
import re
import time
from collections import deque
from typing import Callable, Mapping

from .errors import ApiError

SIGNATURE_VERSION = "EMS1"

HEADER_CLIENT_ID = "x-ems-client-id"
HEADER_TIMESTAMP = "x-ems-timestamp"
HEADER_NONCE = "x-ems-nonce"
HEADER_SIGNATURE = "x-ems-signature"

_NONCE_RE = re.compile(r"^[A-Za-z0-9_-]{16,128}$")


def canonical_string(method: str, path: str, client_id: str, timestamp: str, nonce: str, body: bytes) -> bytes:
    body_hash = hashlib.sha256(body).hexdigest()
    return "\n".join(
        [SIGNATURE_VERSION, method.upper(), path, client_id, timestamp, nonce, body_hash]
    ).encode("utf-8")


def sign(secret: str, method: str, path: str, client_id: str, timestamp: str, nonce: str, body: bytes) -> str:
    message = canonical_string(method, path, client_id, timestamp, nonce, body)
    return hmac.new(secret.encode("utf-8"), message, hashlib.sha256).hexdigest()


def _unauthorized(message: str) -> ApiError:
    return ApiError(401, "unauthorized", message)


class RequestVerifier:
    def __init__(
        self,
        clients: Mapping[str, str],
        max_skew_seconds: int,
        nonce_ttl_seconds: int,
        clock: Callable[[], float] = time.time,
    ):
        self._clients = dict(clients)
        self._max_skew = max_skew_seconds
        self._nonce_ttl = nonce_ttl_seconds
        self._clock = clock
        # (seen_at, key) in insertion order, so expiry is a pop from the left.
        self._nonce_order: deque[tuple[float, str]] = deque()
        self._nonces: set[str] = set()

    def verify(self, method: str, path: str, headers: Mapping[str, str], body: bytes) -> str:
        """Return the authenticated client id, or raise a 401 ApiError."""
        client_id = headers.get(HEADER_CLIENT_ID, "").strip()
        timestamp = headers.get(HEADER_TIMESTAMP, "").strip()
        nonce = headers.get(HEADER_NONCE, "").strip()
        signature = headers.get(HEADER_SIGNATURE, "").strip().lower()

        if not (client_id and timestamp and nonce and signature):
            raise _unauthorized("missing auth headers")
        if not _NONCE_RE.match(nonce):
            raise _unauthorized("malformed nonce")
        try:
            ts = int(timestamp)
        except ValueError:
            raise _unauthorized("malformed timestamp") from None

        now = self._clock()
        if abs(now - ts) > self._max_skew:
            raise _unauthorized("timestamp outside the allowed window; check the system clock")

        secret = self._clients.get(client_id)
        # Unknown clients and bad signatures get the same answer.
        if secret is None:
            raise _unauthorized("invalid credentials")
        expected = sign(secret, method, path, client_id, timestamp, nonce, body)
        if not hmac.compare_digest(expected, signature):
            raise _unauthorized("invalid credentials")

        self._expire_nonces(now)
        key = f"{client_id}:{nonce}"
        if key in self._nonces:
            raise _unauthorized("replayed request")
        self._nonces.add(key)
        self._nonce_order.append((now, key))
        return client_id

    def _expire_nonces(self, now: float) -> None:
        while self._nonce_order and now - self._nonce_order[0][0] > self._nonce_ttl:
            _, key = self._nonce_order.popleft()
            self._nonces.discard(key)


class RateLimiter:
    """Sliding one-minute window per client."""

    def __init__(self, per_minute: int, clock: Callable[[], float] = time.monotonic):
        self._per_minute = per_minute
        self._clock = clock
        self._hits: dict[str, deque[float]] = {}

    def check(self, client_id: str) -> None:
        if self._per_minute <= 0:
            return
        now = self._clock()
        hits = self._hits.setdefault(client_id, deque())
        while hits and now - hits[0] >= 60:
            hits.popleft()
        if len(hits) >= self._per_minute:
            raise ApiError(429, "rate_limited", f"limit of {self._per_minute} requests per minute reached")
        hits.append(now)
