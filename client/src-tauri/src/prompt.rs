//! Prompt for direct Gemini requests. The server builds the same prompt for Claude.

use crate::api::Mode;

pub const SYSTEM_PROMPT: &str = include_str!("../../../server/ems_solver/system_prompt.md");

fn instruction(mode: Mode) -> &'static str {
    // Keep in sync with MODE_INSTRUCTIONS in server/ems_solver/prompts.py.
    match mode {
        Mode::Solution => "",
        Mode::Hint => "Give only a hint or the next step unless a full solution is absolutely necessary.",
        Mode::Checkwork => {
            "Treat the input as an attempted solution. Check it, point out mistakes briefly, and give the corrected result."
        }
    }
}

/// The text block that precedes the problem parts.
pub fn lead(mode: Mode) -> String {
    format!("{}\n\nProblem:", instruction(mode)).trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_instructions_match_server() {
        let server = include_str!("../../../server/ems_solver/prompts.py");
        for mode in [Mode::Hint, Mode::Checkwork] {
            assert!(server.contains(instruction(mode)), "{mode:?} instruction drifted from prompts.py");
        }
    }

    #[test]
    fn lead_text() {
        assert_eq!(lead(Mode::Solution), "Problem:");
        assert!(lead(Mode::Hint).starts_with("Give only a hint"));
        assert!(lead(Mode::Hint).ends_with("\n\nProblem:"));
    }
}
