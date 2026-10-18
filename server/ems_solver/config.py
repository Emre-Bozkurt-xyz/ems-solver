import os
from dataclasses import dataclass, field
from typing import Mapping

EFFORT_LEVELS = ("low", "medium", "high", "xhigh", "max")


def parse_client_secrets(raw: str) -> dict[str, str]:
    """Parse CLIENT_SECRETS, formatted as "client-a:secret1,client-b:secret2"."""
    clients: dict[str, str] = {}
    for pair in raw.split(","):
        client_id, sep, secret = pair.strip().partition(":")
        client_id, secret = client_id.strip(), secret.strip()
        if sep and client_id and secret:
            clients[client_id] = secret
    return clients


@dataclass(frozen=True)
class Settings:
    anthropic_api_key: str
    anthropic_model: str = "claude-sonnet-5-5"
    anthropic_effort: str = "medium"
    anthropic_timeout_seconds: float = 300.0
    max_output_tokens: int = 32000

    client_secrets: dict[str, str] = field(default_factory=dict)
    auth_max_skew_seconds: int = 60
    auth_nonce_ttl_seconds: int = 120
    rate_limit_per_minute: int = 20

    max_body_bytes: int = 20 * 1024 * 1024
    max_parts: int = 20
    max_images: int = 10
    # Claude's per-image limit for base64 images.
    max_image_bytes: int = 5 * 1024 * 1024
    max_text_chars: int = 50_000

    # Comment frames keep proxies (Cloudflare drops idle requests after ~100s)
    # from cutting a stream while the model is still thinking.
    sse_keepalive_seconds: float = 15.0

    def __post_init__(self) -> None:
        if not self.anthropic_api_key:
            raise ValueError("ANTHROPIC_API_KEY is not set")
        if self.anthropic_effort not in EFFORT_LEVELS:
            raise ValueError(f"ANTHROPIC_EFFORT must be one of {', '.join(EFFORT_LEVELS)}")
        # A request is valid for 2 * skew seconds, so nonces must be remembered
        # at least that long or a captured request could be replayed.
        if self.auth_nonce_ttl_seconds < 2 * self.auth_max_skew_seconds:
            raise ValueError("AUTH_NONCE_TTL_SECONDS must be at least 2 * AUTH_MAX_SKEW_SECONDS")

    @classmethod
    def from_env(cls, env: Mapping[str, str] = os.environ) -> "Settings":
        def get(name: str, default: str) -> str:
            return env.get(name, "").strip() or default

        defaults = cls.__dataclass_fields__
        return cls(
            anthropic_api_key=get("ANTHROPIC_API_KEY", ""),
            anthropic_model=get("ANTHROPIC_MODEL", defaults["anthropic_model"].default),
            anthropic_effort=get("ANTHROPIC_EFFORT", defaults["anthropic_effort"].default),
            anthropic_timeout_seconds=float(get("ANTHROPIC_TIMEOUT_SECONDS", "300")),
            max_output_tokens=int(get("MAX_OUTPUT_TOKENS", "32000")),
            client_secrets=parse_client_secrets(env.get("CLIENT_SECRETS", "")),
            auth_max_skew_seconds=int(get("AUTH_MAX_SKEW_SECONDS", "60")),
            auth_nonce_ttl_seconds=int(get("AUTH_NONCE_TTL_SECONDS", "120")),
            rate_limit_per_minute=int(get("RATE_LIMIT_PER_MINUTE", "20")),
            max_body_bytes=int(get("MAX_BODY_BYTES", str(20 * 1024 * 1024))),
            max_parts=int(get("MAX_PARTS", "20")),
            max_images=int(get("MAX_IMAGES", "10")),
            max_image_bytes=int(get("MAX_IMAGE_BYTES", str(5 * 1024 * 1024))),
            max_text_chars=int(get("MAX_TEXT_CHARS", "50000")),
            sse_keepalive_seconds=float(get("SSE_KEEPALIVE_SECONDS", "15")),
        )
