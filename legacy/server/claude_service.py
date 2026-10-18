import os
from anthropic import Anthropic

ANTHROPIC_API_KEY = os.environ.get("ANTHROPIC_API_KEY", "").strip()
if not ANTHROPIC_API_KEY:
    raise RuntimeError("ANTHROPIC_API_KEY is not set")

ANTHROPIC_MODEL = os.environ.get("ANTHROPIC_MODEL", "claude-sonnet-4-6").strip()

client = Anthropic(api_key=ANTHROPIC_API_KEY)

BASE_PROMPT = """
You are a concise but strong problem solver.

Solve the following problem clearly and correctly.
Prefer a compact answer unless the problem obviously needs derivation.
Do not add fluff.
If the problem statement is ambiguous, state the ambiguity briefly and proceed with the most reasonable interpretation.
Make sure to format your answer properly with new lines "\\n" where necessary.
For coding questions, provide only the code without any explanation or comments, and indent properly and cleanly.
Your responses should be in markdown format for written, verbal solutions, and in the relevant programming language for coding problems. But this is really important: **DONT FORMAT CODE IN MARKDOWN, JUST PROVIDE THE RAW CODE**, so no ```python or ``` around code blocks, just the code itself.
If given a problem with description followed by a code block with missing pieces, provide the entire code block with the missing pieces filled in, keeping existing comments and code intact, and do not add any explanation or text outside the code block. So only modify in areas where it says '# YOUR CODE HERE'
If asked to provide an explanation within the code block, make it very concise and to the point, and do not add any additional explanation that isnt necessary.
If asked to provide an invariant within the code block, answer with propositional logic expressions or simple equations, and try not to give sentences. 
When the user's provided content includes multiple questions, only answer the last question. Use the previous questions, whether answered or not, as context to inform your answer to the last question, but do not answer any of the previous questions. However, if the last problem is a code block, you should still provide the full code block replacement, dont only give the solution for the last `# YOUR CODE HERE'. 
""".strip()

MODE_PREFIX = {
    "solution": "",
    "hint": "Give only a hint or the next step unless a full solution is absolutely necessary.",
    "checkwork": "Treat the input as an attempted solution or work. Check it, point out mistakes briefly, and provide the corrected result.",
}


def _extract_text(message) -> str:
    parts = []
    for block in message.content:
        if getattr(block, "type", None) == "text":
            text = getattr(block, "text", "")
            if text:
                parts.append(text)
    return "\n".join(parts).strip()


def solve_with_claude(problem: str, mode: str = "solution") -> str:
    extra = MODE_PREFIX.get(mode, "")
    user_prompt = f"{extra}\n\nProblem:\n{problem}".strip()

    message = client.messages.create(
        model=ANTHROPIC_MODEL,
        max_tokens=4096,
        system=BASE_PROMPT,
        messages=[
            {
                "role": "user",
                "content": user_prompt,
            }
        ],
    )

    text = _extract_text(message)
    if text:
        return text

    return "[Claude returned no text output.]"