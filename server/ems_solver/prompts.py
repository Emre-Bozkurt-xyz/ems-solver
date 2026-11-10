from pathlib import Path

# Shared with the desktop client, which embeds the same file for its Gemini path.
SYSTEM_PROMPT = (Path(__file__).with_name("system_prompt.md")).read_text(encoding="utf-8").strip()

# Appended to the system prompt, so they take precedence over its general guidance.
# Keep each on one line: the client's tests check that its copies match these exactly.
MODE_INSTRUCTIONS = {
    "solution": "",
    "answer": "Answer-only mode: reply with the final answer and nothing else. This overrides the guidance above: no working, explanation, restated question, caveats, notes about how you interpreted the question, alternative answers or Markdown formatting. If the question is ambiguous, silently go with the most likely interpretation. For a calculation, give just the resulting value (with its unit or currency if it has one); for multiple choice, just the chosen option; for code, just the code.",
    "hint": "Give only a hint or the next step unless a full solution is absolutely necessary.",
    "checkwork": "Treat the input as an attempted solution. Check it, point out mistakes briefly, and give the corrected result.",
}

# Repeated after the problem, which is what the model reads last (often a screenshot).
MODE_REMINDERS = {
    "answer": "Reply with the final answer only.",
}


def system_prompt(mode: str) -> str:
    instruction = MODE_INSTRUCTIONS[mode]
    return f"{SYSTEM_PROMPT}\n\n{instruction}" if instruction else SYSTEM_PROMPT


def user_content(mode: str, problem_blocks: list[dict]) -> list[dict]:
    content = [{"type": "text", "text": "Problem:"}, *problem_blocks]
    if reminder := MODE_REMINDERS.get(mode):
        content.append({"type": "text", "text": reminder})
    return content
