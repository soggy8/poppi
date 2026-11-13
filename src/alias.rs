//! User-defined shortcuts from the `[aliases]` config table.
//!
//! ```toml
//! [aliases]
//! gh = "https://github.com/search?q={}"   # a custom search engine
//! up = "kitty -e sudo dnf upgrade"        # a command
//! ```
//!
//! `{}` is replaced by whatever follows the alias. A command without `{}`
//! gets the arguments appended.

use crate::search;
use crate::utils;
use gtk::gio;
use std::process::Command;

#[derive(Debug, Clone, PartialEq)]
pub enum AliasTarget {
    Url(String),
    Command(String),
}

impl AliasTarget {
    pub fn text(&self) -> &str {
        match self {
            AliasTarget::Url(text) | AliasTarget::Command(text) => text,
        }
    }
}

pub fn expand(template: &str, args: &str) -> AliasTarget {
    let template = template.trim();
    let args = args.trim();

    if template.starts_with("http://") || template.starts_with("https://") {
        return AliasTarget::Url(template.replace("{}", &urlencoding::encode(args)));
    }

    // Each argument is quoted, so typed text cannot inject shell syntax into
    // the alias; only the template from the config file is interpreted.
    let quoted = args
        .split_whitespace()
        .map(utils::shell_quote)
        .collect::<Vec<_>>()
        .join(" ");
    let command = if template.contains("{}") {
        template.replace("{}", &quoted)
    } else if quoted.is_empty() {
        template.to_string()
    } else {
        format!("{} {}", template, quoted)
    };
    AliasTarget::Command(command)
}

pub fn run(target: &AliasTarget, context: Option<&gio::AppLaunchContext>) -> Result<(), String> {
    match target {
        AliasTarget::Url(url) => search::open_url(url, context),
        AliasTarget::Command(command) => {
            utils::spawn_detached(Command::new("sh").arg("-c").arg(command))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_aliases_encode_their_arguments() {
        assert_eq!(
            expand("https://github.com/search?q={}", "gtk4 rust"),
            AliasTarget::Url("https://github.com/search?q=gtk4%20rust".into())
        );
        assert_eq!(
            expand("https://mail.example.com/", "ignored"),
            AliasTarget::Url("https://mail.example.com/".into())
        );
    }

    #[test]
    fn command_aliases_quote_their_arguments() {
        assert_eq!(
            expand("kitty -e htop", ""),
            AliasTarget::Command("kitty -e htop".into())
        );
        assert_eq!(
            expand("code", "my project"),
            AliasTarget::Command("code 'my' 'project'".into())
        );
        assert_eq!(
            expand("notify-send {} done", "it's; rm -rf ~"),
            AliasTarget::Command("notify-send 'it'\\''s;' 'rm' '-rf' '~' done".into())
        );
    }
}
