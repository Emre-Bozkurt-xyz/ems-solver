import hashlib
import hmac
import json
import os
import sys
import time
import urllib.error
import urllib.request
import uuid
from pathlib import Path


def safe_print(text: str) -> None:
    if not isinstance(text, str):
        text = str(text)

    try:
        print(text)
    except UnicodeEncodeError:
        fallback = text.encode("ascii", errors="replace").decode("ascii")
        print(fallback)


def load_config() -> dict:
    script_dir = Path(__file__).resolve().parent
    config_path = script_dir / "client_config.json"

    if not config_path.exists():
        raise FileNotFoundError(f"Missing config file: {config_path}")

    with config_path.open("r", encoding="utf-8") as f:
        data = json.load(f)

    required = ["server_url", "client_id", "client_secret"]
    missing = [k for k in required if not str(data.get(k, "")).strip()]
    if missing:
        raise ValueError(f"Missing required config fields: {', '.join(missing)}")

    return data


def build_prompt(problem: str, mode: str) -> str:
    base_prompt = """
You are a concise but strong problem solver.

Solve the following problem clearly and correctly.
Prefer a compact answer unless the problem obviously needs derivation.
Do not add fluff.
If the problem statement is ambiguous, state the ambiguity briefly and proceed with the most reasonable interpretation.
Make sure to format your answer properly with new lines "\\n" where necessary.
For coding questions, provide only the code without any explanation or comments, and indent properly and cleanly.
Your responses should be in markdown format for written solutions, and in the relevant programming language for coding problems. DONT FORMAT CODE IN MARKDOWN, JUST PROVIDE THE RAW CODE.
If given a problem with description followed by a code block with missing pieces, provide the entire code block with the missing pieces filled in, keeping existing comments and code intact, and do not add any explanation or text outside the code block.
""".strip()

    mode_prefix = {
        "solution": "",
        "hint": "Give only a hint or the next step unless a full solution is absolutely necessary.",
        "checkwork": "Treat the input as an attempted solution or work. Check it, point out mistakes briefly, and provide the corrected result.",
    }

    extra = mode_prefix.get(mode, "")
    return f"{base_prompt}\n\n{extra}\n\nProblem:\n{problem}".strip()


def extract_gemini_text(resp_json: dict) -> str:
    try:
        candidates = resp_json.get("candidates", [])
        if not candidates:
            return ""

        parts = candidates[0].get("content", {}).get("parts", [])
        texts = [p.get("text", "") for p in parts if p.get("text")]
        return "\n".join(texts).strip()
    except Exception:
        return ""


def call_gemini_fallback(problem: str, mode: str, config: dict) -> str:
    gemini_enabled = bool(config.get("fallback_to_gemini", True))
    gemini_api_key = str(config.get("gemini_api_key", "")).strip()
    gemini_model = str(config.get("gemini_model", "gemini-3-flash-preview")).strip()
    gemini_timeout = float(config.get("gemini_timeout_seconds", 120))

    if not gemini_enabled:
        return "[Server unavailable, and Gemini fallback is disabled.]"

    if not gemini_api_key:
        return "[Server unavailable, and Gemini fallback API key is missing.]"

    prompt = build_prompt(problem, mode)

    url = (
        f"https://generativelanguage.googleapis.com/v1beta/models/"
        f"{gemini_model}:generateContent?key={gemini_api_key}"
    )

    body_obj = {
        "contents": [
            {
                "parts": [
                    {
                        "text": prompt
                    }
                ]
            }
        ]
    }

    raw_body = json.dumps(body_obj, ensure_ascii=False).encode("utf-8")

    req = urllib.request.Request(
        url=url,
        data=raw_body,
        method="POST",
        headers={
            "Content-Type": "application/json; charset=utf-8",
            "User-Agent": "EspansoSolver/1.0",
        },
    )

    try:
        with urllib.request.urlopen(req, timeout=gemini_timeout) as resp:
            data = json.loads(resp.read().decode("utf-8"))
            text = extract_gemini_text(data)
            if text:
                return text
            return "[Gemini fallback returned no text output.]"
    except urllib.error.HTTPError as e:
        try:
            err = e.read().decode("utf-8", errors="replace")
            return f"[Gemini fallback HTTP {e.code}] {err}"
        except Exception:
            return f"[Gemini fallback HTTP {e.code}]"
    except urllib.error.URLError as e:
        return f"[Gemini fallback connection error: {e}]"
    except Exception as e:
        return f"[Gemini fallback error: {e}]"


def should_fallback_on_http_status(status_code: int) -> bool:
    return status_code >= 500


def try_server(problem: str, mode: str, config: dict) -> str:
    server_url = str(config["server_url"]).strip()
    client_id = str(config["client_id"]).strip()
    client_secret = str(config["client_secret"]).strip()
    timeout_seconds = float(config.get("timeout_seconds", 120))

    body_obj = {
        "text": problem,
        "mode": mode,
    }

    raw_body = json.dumps(body_obj, separators=(",", ":"), ensure_ascii=False).encode("utf-8")

    timestamp = str(int(time.time()))
    nonce = uuid.uuid4().hex

    mac = hmac.new(client_secret.encode("utf-8"), digestmod=hashlib.sha256)
    mac.update(client_id.encode("utf-8"))
    mac.update(b"\n")
    mac.update(timestamp.encode("utf-8"))
    mac.update(b"\n")
    mac.update(nonce.encode("utf-8"))
    mac.update(b"\n")
    mac.update(raw_body)
    signature = mac.hexdigest()

    req = urllib.request.Request(
        url=server_url,
        data=raw_body,
        method="POST",
        headers={
            "User-Agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/135.0.0.0 Safari/537.36",
            "Content-Type": "application/json; charset=utf-8",
            "X-Client-Id": client_id,
            "X-Timestamp": timestamp,
            "X-Nonce": nonce,
            "X-Signature": signature,
        },
    )

    with urllib.request.urlopen(req, timeout=timeout_seconds) as resp:
        data = json.loads(resp.read().decode("utf-8"))
        result = data.get("result", "")
        if result.strip():
            return result.strip()
        return "[Server returned no result.]"


def main():
    try:
        sys.stdout.reconfigure(encoding="utf-8")
    except Exception:
        pass

    problem = os.environ.get("ESPANSO_CLIP", "").strip()
    if not problem:
        safe_print("[No clipboard text found. Copy a problem first.]")
        return

    try:
        config = load_config()
    except Exception as e:
        safe_print(f"[Config error: {e}]")
        return

    default_mode = str(config.get("mode", "solution")).strip() or "solution"
    mode = os.environ.get("ESPANSO_MODE", "").strip() or default_mode

    try:
        result = try_server(problem, mode, config)
        safe_print(result)
        return

    except urllib.error.HTTPError as e:
        if should_fallback_on_http_status(e.code):
            fallback_result = call_gemini_fallback(problem, mode, config)
            safe_print(fallback_result)
            return

        try:
            err = e.read().decode("utf-8", errors="replace")
            safe_print(f"[HTTP {e.code}] {err}")
        except Exception:
            safe_print(f"[HTTP {e.code}]")
        return

    except urllib.error.URLError:
        fallback_result = call_gemini_fallback(problem, mode, config)
        safe_print(fallback_result)
        return

    except TimeoutError:
        fallback_result = call_gemini_fallback(problem, mode, config)
        safe_print(fallback_result)
        return

    except Exception as e:
        message = str(e).lower()
        if "timed out" in message or "timeout" in message:
            fallback_result = call_gemini_fallback(problem, mode, config)
            safe_print(fallback_result)
        else:
            safe_print(f"[Client error: {e}]")


if __name__ == "__main__":
    main()