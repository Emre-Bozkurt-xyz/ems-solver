import hmac
import hashlib
import os
import time
from typing import Dict, Tuple

# Format:
# CLIENT_SECRETS="emre-pc:supersecret1,friend-laptop:supersecret2"
def load_client_secrets() -> Dict[str, str]:
    raw = os.environ.get("CLIENT_SECRETS", "").strip()
    mapping: Dict[str, str] = {}

    if not raw:
        return mapping

    for pair in raw.split(","):
        pair = pair.strip()
        if not pair:
            continue
        if ":" not in pair:
            continue
        client_id, secret = pair.split(":", 1)
        client_id = client_id.strip()
        secret = secret.strip()
        if client_id and secret:
            mapping[client_id] = secret

    return mapping


CLIENTS = load_client_secrets()

MAX_SKEW_SECONDS = int(os.environ.get("AUTH_MAX_SKEW_SECONDS", "60"))
NONCE_TTL_SECONDS = int(os.environ.get("AUTH_NONCE_TTL_SECONDS", "120"))

# In-memory replay cache. Fine for one small private service.
USED_NONCES: Dict[str, int] = {}


def _cleanup_nonces(now: int) -> None:
    expired = [k for k, ts in USED_NONCES.items() if now - ts > NONCE_TTL_SECONDS]
    for k in expired:
        USED_NONCES.pop(k, None)


def verify_request(
    headers,
    raw_body: bytes,
) -> Tuple[bool, str]:
    client_id = headers.get("x-client-id", "").strip()
    timestamp = headers.get("x-timestamp", "").strip()
    nonce = headers.get("x-nonce", "").strip()
    signature = headers.get("x-signature", "").strip()

    if not client_id or not timestamp or not nonce or not signature:
        return False, "missing auth headers"

    secret = CLIENTS.get(client_id)
    if not secret:
        return False, "unknown client"

    try:
        ts = int(timestamp)
    except ValueError:
        return False, "invalid timestamp"

    now = int(time.time())
    if abs(now - ts) > MAX_SKEW_SECONDS:
        return False, "timestamp outside allowed window"

    _cleanup_nonces(now)

    nonce_key = f"{client_id}:{nonce}"
    if nonce_key in USED_NONCES:
        return False, "replay detected"

    mac = hmac.new(secret.encode("utf-8"), digestmod=hashlib.sha256)
    mac.update(client_id.encode("utf-8"))
    mac.update(b"\n")
    mac.update(timestamp.encode("utf-8"))
    mac.update(b"\n")
    mac.update(nonce.encode("utf-8"))
    mac.update(b"\n")
    mac.update(raw_body)

    expected = mac.hexdigest()
    
    if not hmac.compare_digest(expected, signature):
        return False, "bad signature"

    USED_NONCES[nonce_key] = now
    return True, ""