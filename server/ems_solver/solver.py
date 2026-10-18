import logging
from dataclasses import dataclass
from typing import AsyncIterator, Protocol, Union

import anthropic

from .config import Settings
from .errors import ApiError
from .prompts import SYSTEM_PROMPT, user_content

log = logging.getLogger(__name__)

# Server-side refusal fallback: a declined request is re-run on the model
# Anthropic recommends for that refusal category, inside the same call.
FALLBACK_BETA = "server-side-fallback-2026-07-01"


@dataclass
class Usage:
    input_tokens: int
    output_tokens: int


@dataclass
class TextDelta:
    text: str


@dataclass
class Completed:
    text: str
    model: str
    stop_reason: str | None
    usage: Usage


SolveEvent = Union[TextDelta, Completed]


class Solver(Protocol):
    def stream(self, mode: str, problem_blocks: list[dict]) -> AsyncIterator[SolveEvent]:
        """Yield TextDelta events, then exactly one Completed. Raise ApiError on failure."""
        ...


def _upstream_error(exc: anthropic.APIError) -> ApiError:
    # Every upstream failure is retryable from the client's point of view:
    # whatever went wrong between us and Claude, another provider may still answer.
    if isinstance(exc, anthropic.APIConnectionError):
        return ApiError(502, "upstream_unavailable", "could not reach Claude", retryable=True)
    if isinstance(exc, anthropic.APIStatusError):
        if exc.status_code == 429:
            return ApiError(503, "upstream_rate_limited", "Claude rate limit reached", retryable=True)
        return ApiError(502, "upstream_error", f"Claude API returned {exc.status_code}", retryable=True)
    return ApiError(502, "upstream_error", "Claude API error", retryable=True)


class ClaudeSolver:
    def __init__(self, settings: Settings, client: anthropic.AsyncAnthropic | None = None):
        self._settings = settings
        self._client = client or anthropic.AsyncAnthropic(
            api_key=settings.anthropic_api_key,
            timeout=settings.anthropic_timeout_seconds,
        )

    async def stream(self, mode: str, problem_blocks: list[dict]) -> AsyncIterator[SolveEvent]:
        s = self._settings
        try:
            async with self._client.beta.messages.stream(
                model=s.anthropic_model,
                max_tokens=s.max_output_tokens,
                system=SYSTEM_PROMPT,
                messages=[{"role": "user", "content": user_content(mode, problem_blocks)}],
                output_config={"effort": s.anthropic_effort},
                betas=[FALLBACK_BETA],
                fallbacks="default",
            ) as stream:
                async for event in stream:
                    if event.type == "text" and event.text:
                        yield TextDelta(event.text)
                message = await stream.get_final_message()
        except anthropic.APIError as exc:
            log.warning("claude request failed: %r", exc)
            raise _upstream_error(exc) from exc

        if message.stop_reason == "refusal":
            category = getattr(message.stop_details, "category", None)
            log.info("claude refused (category=%s)", category)
            raise ApiError(
                422,
                "refused",
                f"Claude declined this request{f' ({category})' if category else ''}",
                retryable=True,
            )

        # After a mid-stream fallback the text blocks are the partial answer plus
        # its continuation, so joining them gives the same text that was streamed.
        text = "".join(block.text for block in message.content if block.type == "text")
        yield Completed(
            text=text.strip(),
            model=message.model,
            stop_reason=message.stop_reason,
            usage=Usage(message.usage.input_tokens, message.usage.output_tokens),
        )
