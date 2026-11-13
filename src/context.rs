//! Text a question is about: what is selected in another app ("explain
//! this") or on the clipboard ("summarize my clipboard"). It is only sent
//! when the question refers to it, and the row says so first, so nothing
//! reaches an online AI unnoticed.

/// Longest text sent along, in characters.
const MAX_CHARS: usize = 6000;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Source {
    Selection,
    Clipboard,
}

impl Source {
    pub fn describe(self) -> &'static str {
        match self {
            Source::Selection => "with your selected text",
            Source::Clipboard => "with the text on your clipboard",
        }
    }
}

/// Which text the question is about, if it refers to one.
pub fn wanted(question: &str) -> Option<Source> {
    let words: Vec<String> = question
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect();
    let has = |w: &str| words.iter().any(|x| x == w);
    if has("clipboard") || has("copied") {
        Some(Source::Clipboard)
    } else if has("this") || has("selected") || has("selection") || has("highlighted") {
        Some(Source::Selection)
    } else {
        None
    }
}

/// The question as sent to the model, with the text it is about.
pub fn attach(question: &str, context: Option<&str>) -> String {
    let Some(text) = context.map(str::trim).filter(|t| !t.is_empty()) else {
        return question.to_string();
    };
    let text: String = text.chars().take(MAX_CHARS).collect();
    format!(
        "{}\n\nThe text this is about:\n\"\"\"\n{}\n\"\"\"",
        question, text
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn questions_that_refer_to_a_text() {
        assert_eq!(wanted("explain this?"), Some(Source::Selection));
        assert_eq!(
            wanted("translate the selected text to German"),
            Some(Source::Selection)
        );
        assert_eq!(wanted("summarize my clipboard"), Some(Source::Clipboard));
        assert_eq!(
            wanted("what did I just copy? fix the copied code"),
            Some(Source::Clipboard)
        );
        assert_eq!(wanted("what is thistle?"), None);
        assert_eq!(wanted("capital of France?"), None);
    }

    #[test]
    fn the_text_is_quoted_after_the_question_and_capped() {
        assert_eq!(attach("explain this?", None), "explain this?");
        assert_eq!(attach("explain this?", Some("  ")), "explain this?");
        assert_eq!(
            attach("explain this?", Some(" fn main() {} ")),
            "explain this?\n\nThe text this is about:\n\"\"\"\nfn main() {}\n\"\"\""
        );
        let long = "x".repeat(MAX_CHARS + 50);
        let sent = attach("q", Some(&long));
        let quoted = sent.split("\"\"\"\n").nth(1).unwrap();
        assert_eq!(quoted.matches('x').count(), MAX_CHARS);
    }
}
