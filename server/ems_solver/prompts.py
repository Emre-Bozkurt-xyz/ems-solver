from pathlib import Path

# Shared with the desktop client, which embeds the same file for its Gemini path.
SYSTEM_PROMPT = (Path(__file__).with_name("system_prompt.md")).read_text(encoding="utf-8").strip()

MODE_INSTRUCTIONS = {
    "solution": "",
    "hint": "Give only a hint or the next step unless a full solution is absolutely necessary.",
    "checkwork": "Treat the input as an attempted solution. Check it, point out mistakes briefly, and give the corrected result.",
}


def user_content(mode: str, problem_blocks: list[dict]) -> list[dict]:
    lead = f"{MODE_INSTRUCTIONS[mode]}\n\nProblem:".strip()
    return [{"type": "text", "text": lead}, *problem_blocks]
