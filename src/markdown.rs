//! A small markdown-to-Pango converter for AI answers.
//!
//! Handles what a short answer actually contains: paragraphs, `code`,
//! fenced code blocks, **bold**, *italic*, headings and bullet lists.
//! Anything unrecognised, or not yet closed while text is still streaming
//! in, is shown literally. Output is always well-formed Pango markup.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    /// Paragraphs, lists and inline styles as Pango markup.
    Prose(String),
    /// A fenced code block (` ```lang ... ``` `) as raw, unescaped code.
    Code {
        language: Option<String>,
        code: String,
    },
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Find `marker` in `chars` at or after `from`.
fn find(chars: &[char], from: usize, marker: &[char]) -> Option<usize> {
    if marker.is_empty() || chars.len() < marker.len() {
        return None;
    }
    (from..=chars.len() - marker.len()).find(|&i| chars[i..i + marker.len()] == *marker)
}

fn text_of(chars: &[char]) -> String {
    chars.iter().collect()
}

fn inline(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::new();
    let mut plain = String::new();
    let mut i = 0;

    let flush = |plain: &mut String, out: &mut String| {
        out.push_str(&escape(plain));
        plain.clear();
    };

    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();

        // `code`
        if c == '`' {
            if let Some(end) = find(&chars, i + 1, &['`']) {
                if end > i + 1 {
                    flush(&mut plain, &mut out);
                    out.push_str(&format!(
                        "<tt>{}</tt>",
                        escape(&text_of(&chars[i + 1..end]))
                    ));
                    i = end + 1;
                    continue;
                }
            }
        }

        // **bold**
        if c == '*' && next == Some('*') {
            if let Some(end) = find(&chars, i + 2, &['*', '*']) {
                if end > i + 2 && !chars[i + 2].is_whitespace() {
                    flush(&mut plain, &mut out);
                    out.push_str(&format!("<b>{}</b>", inline(&text_of(&chars[i + 2..end]))));
                    i = end + 2;
                    continue;
                }
            }
        }

        // *italic* or _italic_ (underscores only at word boundaries, so
        // snake_case_names stay intact)
        if c == '*' || c == '_' {
            let opens = next.is_some_and(|n| !n.is_whitespace() && n != c)
                && (c == '*' || i == 0 || !chars[i - 1].is_alphanumeric());
            if opens {
                if let Some(end) = find(&chars, i + 1, &[c]) {
                    let closes = !chars[end - 1].is_whitespace()
                        && (c == '*' || !chars.get(end + 1).is_some_and(|n| n.is_alphanumeric()));
                    if closes {
                        flush(&mut plain, &mut out);
                        out.push_str(&format!("<i>{}</i>", escape(&text_of(&chars[i + 1..end]))));
                        i = end + 1;
                        continue;
                    }
                }
            }
        }

        plain.push(c);
        i += 1;
    }
    flush(&mut plain, &mut out);
    out
}

fn prose_line(line: &str) -> String {
    let trimmed = line.trim_start();
    let indent = &line[..line.len() - trimmed.len()];
    let heading = trimmed.trim_start_matches('#');
    if heading.len() < trimmed.len() && heading.starts_with(' ') {
        format!("<b>{}</b>", inline(heading.trim()))
    } else if let Some(item) = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
        .or_else(|| trimmed.strip_prefix("+ "))
    {
        format!("{}  •  {}", indent, inline(item))
    } else {
        format!("{}{}", indent, inline(trimmed))
    }
}

#[cfg(test)]
pub fn to_pango(markdown: &str) -> String {
    let mut lines = Vec::new();
    let mut in_code = false;

    for line in markdown.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if in_code {
            lines.push(format!("<tt>{}</tt>", escape(line)));
        } else {
            lines.push(prose_line(line));
        }
    }
    lines.join("\n")
}

fn flush_prose(prose: &mut Vec<String>, out: &mut Vec<Block>) {
    while prose.first().is_some_and(|l| l.trim().is_empty()) {
        prose.remove(0);
    }
    while prose.last().is_some_and(|l| l.trim().is_empty()) {
        prose.pop();
    }
    if !prose.is_empty() {
        out.push(Block::Prose(prose.join("\n")));
        prose.clear();
    }
}

/// Split markdown into Pango prose segments and raw fenced code blocks.
pub fn blocks(markdown: &str) -> Vec<Block> {
    let mut out = Vec::new();
    let mut prose = Vec::new();
    let mut code = Vec::new();
    let mut language: Option<String> = None;
    let mut in_code = false;

    for line in markdown.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            if in_code {
                let text = code.join("\n");
                code.clear();
                if !text.trim().is_empty() {
                    out.push(Block::Code {
                        language: language.take(),
                        code: text,
                    });
                } else {
                    language = None;
                }
                in_code = false;
            } else {
                flush_prose(&mut prose, &mut out);
                let tag = trimmed.trim_start_matches('`').trim();
                language = (!tag.is_empty()).then(|| tag.to_string());
                in_code = true;
            }
            continue;
        }
        if in_code {
            code.push(line.to_string());
        } else {
            prose.push(prose_line(line));
        }
    }

    if in_code {
        let text = code.join("\n");
        if !text.trim().is_empty() {
            out.push(Block::Code {
                language,
                code: text,
            });
        }
    } else {
        flush_prose(&mut prose, &mut out);
    }
    out
}

/// Raw text of the last fenced code block in `markdown`, if any.
pub fn last_code_block(markdown: &str) -> Option<String> {
    blocks(markdown)
        .into_iter()
        .rev()
        .find_map(|block| match block {
            Block::Code { code, .. } => Some(code),
            Block::Prose(_) => None,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_is_escaped() {
        assert_eq!(to_pango("a < b && c > d"), "a &lt; b &amp;&amp; c &gt; d");
    }

    #[test]
    fn inline_styles() {
        assert_eq!(to_pango("use `tar -xzf` now"), "use <tt>tar -xzf</tt> now");
        assert_eq!(
            to_pango("this is **important**."),
            "this is <b>important</b>."
        );
        assert_eq!(
            to_pango("an *emphasised* word"),
            "an <i>emphasised</i> word"
        );
        assert_eq!(
            to_pango("an _emphasised_ word"),
            "an <i>emphasised</i> word"
        );
        assert_eq!(
            to_pango("**bold with `code`**"),
            "<b>bold with <tt>code</tt></b>"
        );
        assert_eq!(to_pango("`Vec<T>` & co"), "<tt>Vec&lt;T&gt;</tt> &amp; co");
    }

    #[test]
    fn things_that_are_not_markup_stay_literal() {
        assert_eq!(
            to_pango("snake_case_name and other_name"),
            "snake_case_name and other_name"
        );
        assert_eq!(to_pango("2 * 3 * 4"), "2 * 3 * 4");
        assert_eq!(to_pango("a ** b"), "a ** b");
        // Unclosed markers, as seen while an answer is still streaming.
        assert_eq!(to_pango("this is **impor"), "this is **impor");
        assert_eq!(to_pango("run `tar -x"), "run `tar -x");
    }

    #[test]
    fn blocks() {
        assert_eq!(to_pango("# Title\ntext"), "<b>Title</b>\ntext");
        assert_eq!(to_pango("- one\n* two"), "  •  one\n  •  two");
        assert_eq!(
            to_pango("before\n```rust\nlet x = a < b;\n```\nafter"),
            "before\n<tt>let x = a &lt; b;</tt>\nafter"
        );
        // An unterminated fence keeps the rest as code.
        assert_eq!(to_pango("```\n**not bold**"), "<tt>**not bold**</tt>");
        assert_eq!(to_pango("#hashtag"), "#hashtag");
    }

    #[test]
    fn structured_blocks_separate_prose_and_code() {
        let md = "Use `tar`:\n\n```bash\ntar -xzf a.tar.gz\n```\n\nOr with `<dir>`:";
        assert_eq!(
            super::blocks(md),
            vec![
                Block::Prose("Use <tt>tar</tt>:".into()),
                Block::Code {
                    language: Some("bash".into()),
                    code: "tar -xzf a.tar.gz".into(),
                },
                Block::Prose("Or with <tt>&lt;dir&gt;</tt>:".into()),
            ]
        );
        assert_eq!(last_code_block(md).as_deref(), Some("tar -xzf a.tar.gz"));
        assert_eq!(last_code_block("plain text only"), None);

        // Unclosed fence while streaming still emits a code block once non-empty.
        assert_eq!(
            super::blocks("Intro\n```rust\nlet x = 1 < 2;"),
            vec![
                Block::Prose("Intro".into()),
                Block::Code {
                    language: Some("rust".into()),
                    code: "let x = 1 < 2;".into(),
                },
            ]
        );
        assert_eq!(
            super::blocks("Intro\n```rust"),
            vec![Block::Prose("Intro".into())]
        );
    }
}
