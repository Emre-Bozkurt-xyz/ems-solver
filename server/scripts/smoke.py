"""Send a signed request to an ems-solver server. Standard library only.

    EMS_CLIENT_ID=... EMS_CLIENT_SECRET=... python scripts/smoke.py \\
        --url http://127.0.0.1:8011 --text "What is 17 * 23?" [--image shot.png] [--stream]

Doubles as the reference implementation of the request signature (see docs/api.md).
"""

import argparse
import base64
import hashlib
import hmac
import json
import mimetypes
import os
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid


def signed_request(base_url: str, path: str, payload: dict, client_id: str, secret: str) -> urllib.request.Request:
    body = json.dumps(payload).encode("utf-8")
    timestamp = str(int(time.time()))
    nonce = uuid.uuid4().hex
    full_path = urllib.parse.urlsplit(base_url).path.rstrip("/") + path
    canonical = "\n".join(
        ["EMS1", "POST", full_path, client_id, timestamp, nonce, hashlib.sha256(body).hexdigest()]
    )
    signature = hmac.new(secret.encode(), canonical.encode(), hashlib.sha256).hexdigest()
    return urllib.request.Request(
        base_url.rstrip("/") + path,
        data=body,
        method="POST",
        headers={
            "Content-Type": "application/json",
            "User-Agent": "ems-solver-smoke/0.1",
            "X-Ems-Client-Id": client_id,
            "X-Ems-Timestamp": timestamp,
            "X-Ems-Nonce": nonce,
            "X-Ems-Signature": signature,
        },
    )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--url", default="http://127.0.0.1:8011")
    parser.add_argument("--text", action="append", default=[])
    parser.add_argument("--image", action="append", default=[])
    parser.add_argument("--mode", default="solution")
    parser.add_argument("--stream", action="store_true")
    args = parser.parse_args()

    parts = [{"type": "text", "text": t} for t in args.text]
    for path in args.image:
        media_type = mimetypes.guess_type(path)[0] or "image/png"
        with open(path, "rb") as f:
            parts.append({"type": "image", "media_type": media_type, "data": base64.b64encode(f.read()).decode()})
    if not parts:
        parser.error("give at least one --text or --image")

    request = signed_request(
        args.url,
        "/v1/solve/stream" if args.stream else "/v1/solve",
        {"mode": args.mode, "parts": parts},
        os.environ["EMS_CLIENT_ID"],
        os.environ["EMS_CLIENT_SECRET"],
    )
    started = time.monotonic()
    try:
        with urllib.request.urlopen(request, timeout=300) as response:
            if not args.stream:
                print(json.dumps(json.loads(response.read()), indent=2, ensure_ascii=False))
            else:
                event = None
                for raw in response:
                    line = raw.decode("utf-8").rstrip("\n")
                    if line.startswith("event: "):
                        event = line[7:]
                    elif line.startswith("data: "):
                        data = json.loads(line[6:])
                        if event == "delta":
                            print(data["text"], end="", flush=True)
                        else:
                            data.pop("result", None)
                            print(f"\n[{event}] {json.dumps(data)}")
                    elif line.startswith(":"):
                        print(f"[{time.monotonic() - started:.0f}s keepalive]", file=sys.stderr)
    except urllib.error.HTTPError as exc:
        print(f"HTTP {exc.code}: {exc.read().decode('utf-8', 'replace')}")
        return 1
    print(f"({time.monotonic() - started:.1f}s)", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
