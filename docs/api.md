# ems-solver server API (v1)

All bodies are JSON (UTF-8). Every endpoint except `/healthz` requires a signed request.

## Authentication

Each client has an id and a shared secret (`CLIENT_SECRETS=id:secret,...` on the server).
Sign every request with HMAC-SHA256 over this canonical string, lines joined by `\n`:

```
EMS1
{METHOD}                e.g. POST
{path}                  e.g. /v1/solve/stream  (exactly as sent, no query string)
{client_id}
{timestamp}             unix seconds
{nonce}                 16-128 chars of [A-Za-z0-9_-], unique per request
{sha256_hex(body)}      lowercase hex of the exact body bytes sent
```

Headers:

| Header | Value |
|---|---|
| `X-Ems-Client-Id` | client id |
| `X-Ems-Timestamp` | the timestamp above |
| `X-Ems-Nonce` | the nonce above |
| `X-Ems-Signature` | lowercase hex HMAC-SHA256(secret, canonical string) |

The timestamp must be within 60 s of the server clock, and a nonce can be used once.
`server/scripts/smoke.py` is a reference implementation.

## Request body

`POST /v1/solve` and `POST /v1/solve/stream` take the same body:

```json
{
  "mode": "solution",
  "parts": [
    { "type": "text", "text": "Earlier question, for context" },
    { "type": "image", "media_type": "image/png", "data": "<base64, no data: prefix>" },
    { "type": "text", "text": "The question to answer" }
  ]
}
```

- `mode`: `solution` (default), `hint`, or `checkwork`.
- `parts`: 1-20 items, sent to the model in order. When there are several questions,
  the model answers the last one and uses the rest as context.
- Images: `image/png`, `image/jpeg`, `image/gif`, `image/webp`; at most 10, 5 MB each after
  decoding. Clients should downscale screenshots to about 1568 px on the long edge first;
  the model doesn't use more detail than that, and it keeps requests small.
- Limits: 20 MB body, 50,000 characters of text in total, 20 requests per minute per client.

## `POST /v1/solve/stream` (recommended)

Server-sent events. Use this endpoint even if you only want the final answer: the
connection carries keepalive comments (`: keepalive`) every 15 s while the model thinks,
so proxies such as Cloudflare (which times out idle requests at ~100 s) don't cut it.

```
event: delta
data: {"text": "partial answer text"}

event: done
data: {"result": "full answer", "model": "claude-sonnet-5-5", "stop_reason": "end_turn",
       "usage": {"input_tokens": 576, "output_tokens": 3}}
```

or, if something fails after the stream started:

```
event: error
data: {"code": "refused", "message": "...", "retryable": true}
```

After an `error` event, discard any text from earlier `delta` events. Errors found before
the stream starts (auth, validation, limits) are returned as plain JSON with an HTTP
status, as described under Errors.

## `POST /v1/solve`

The same request, answered in one response:

```json
{ "ok": true, "result": "...", "model": "...", "stop_reason": "end_turn",
  "usage": { "input_tokens": 0, "output_tokens": 0 } }
```

`stop_reason` is `max_tokens` when the answer was cut off.

## Errors

```json
{ "ok": false, "error": { "code": "invalid_request", "message": "...", "retryable": false } }
```

`retryable: true` means the request was fine but the server couldn't answer it, so a
client may fall back to another provider (such as Gemini).

| Status | Code | Retryable | Meaning |
|---|---|---|---|
| 400 | `invalid_request` | no | Malformed body, bad part, too many parts or images |
| 401 | `unauthorized` | no | Missing, wrong or replayed signature, or clock skew |
| 413 | `payload_too_large` | no | Body, text or image over the limit |
| 429 | `rate_limited` | no | Per-client request limit reached |
| 422 | `refused` | yes | Claude declined the request |
| 502 | `upstream_error`, `upstream_unavailable` | yes | Claude API error or unreachable |
| 503 | `upstream_rate_limited` | yes | Claude API rate limit |
| 500 | `internal_error` | yes | Unexpected server error |

A network failure, or a 5xx without this JSON body (for example from a proxy), should also
be treated as retryable.

## `GET /healthz`

`{"ok": true}`, no auth.
