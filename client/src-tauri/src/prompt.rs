//! Prompt for direct Gemini requests. The server builds the same prompt for Claude.

use crate::api::Mode;

const BASE_PROMPT: &str = include_str!("../../../server/ems_solver/system_prompt.md");

/// Appended to the system prompt, so it takes precedence over the general guidance.
fn instruction(mode: Mode) -> &'static str {
    // Keep in sync with MODE_INSTRUCTIONS in server/ems_solver/prompts.py.
    match mode {
        Mode::Solution => "",
        Mode::Answer => {
            "Answer-only mode: reply with only the final answer, wrapped in <answer></answer> tags, for example <answer>42</answer>. Only the text inside the tags is shown to the user. Inside the tags: no working, explanation, caveats, notes about how you interpreted the question, alternative answers or Markdown formatting. If the question is ambiguous, go with the most likely interpretation. For a calculation, give just the resulting value (with its unit or currency if it has one); for multiple choice, just the chosen option; for code, just the code."
        }
        Mode::Hint => "Give only a hint or the next step unless a full solution is absolutely necessary.",
        Mode::Checkwork => {
            "Treat the input as an attempted solution. Check it, point out mistakes briefly, and give the corrected result."
        }
    }
}

/// Repeated after the problem, which is what the model reads last (often a screenshot).
pub fn reminder(mode: Mode) -> Option<&'static str> {
    // Keep in sync with MODE_REMINDERS in server/ems_solver/prompts.py.
    match mode {
        Mode::Answer => Some("Reply with the final answer only, inside <answer></answer> tags."),
        _ => None,
    }
}

/// In answer mode, keeps only the last <answer> block; the model sometimes adds working anyway.
/// Same rule as final_text in server/ems_solver/prompts.py.
pub fn final_text(mode: Mode, text: &str) -> String {
    if mode != Mode::Answer {
        return text.to_string();
    }
    match text.rfind("<answer>") {
        Some(start) => {
            let rest = &text[start + "<answer>".len()..];
            rest.split("</answer>").next().unwrap_or(rest).trim().to_string()
        }
        None => text.trim().to_string(),
    }
}

pub fn system_prompt(mode: Mode) -> String {
    let base = BASE_PROMPT.trim();
    match instruction(mode) {
        "" => base.to_string(),
        instruction => format!("{base}\n\n{instruction}"),
    }
}

/// The text block that precedes the problem parts.
pub const LEAD: &str = "Problem:";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_texts_match_server() {
        let server = include_str!("../../../server/ems_solver/prompts.py");
        for mode in Mode::ALL {
            assert!(server.contains(instruction(mode)), "{mode:?} instruction drifted from prompts.py");
            if let Some(reminder) = reminder(mode) {
                assert!(server.contains(reminder), "{mode:?} reminder drifted from prompts.py");
            }
        }
    }

    #[test]
    fn system_prompt_per_mode() {
        assert_eq!(system_prompt(Mode::Solution), BASE_PROMPT.trim());
        assert!(system_prompt(Mode::Answer).ends_with("for code, just the code."));
        assert!(system_prompt(Mode::Answer).contains("<answer>42</answer>"));
        assert!(system_prompt(Mode::Hint).starts_with("You are a concise but strong problem solver."));
        assert_eq!(reminder(Mode::Hint), None);
    }

    #[test]
    fn final_text_keeps_the_last_answer_block() {
        let cases = [
            ("Payoff is -1100.\n\n<answer>-$1100</answer>\n\n(Profit: 120.44)", "-$1100"),
            ("<answer>1</answer> wait, <answer>\n2\n</answer>", "2"),
            ("<answer>cut off mid", "cut off mid"),
            ("  no tags at all  ", "no tags at all"),
        ];
        for (raw, expected) in cases {
            assert_eq!(final_text(Mode::Answer, raw), expected);
            assert_eq!(final_text(Mode::Solution, raw), raw);
        }
    }
}
