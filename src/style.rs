use crate::config::{Config, ThemeConfig};
use gtk::gdk;

const TEMPLATE: &str = include_str!("style.css");

thread_local! {
    static PROVIDER: gtk::CssProvider = gtk::CssProvider::new();
    static INSTALLED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub fn is_valid_color(color: &str) -> bool {
    let color = color.trim();
    // Reject anything that could break out of the declaration it is pasted into.
    !color.is_empty()
        && !color.contains([';', '{', '}'])
        && gdk::RGBA::parse(color).is_ok()
}

fn color_or(color: &str, fallback: &str) -> String {
    if is_valid_color(color) {
        color.trim().to_string()
    } else {
        eprintln!("Ignoring invalid color {:?}, using {}", color, fallback);
        fallback.to_string()
    }
}

pub fn build_css(config: &Config) -> String {
    let defaults = ThemeConfig::default();
    let theme = &config.theme;
    TEMPLATE
        .replace(
            "{{background}}",
            &color_or(&theme.background_color, &defaults.background_color),
        )
        .replace("{{text}}", &color_or(&theme.text_color, &defaults.text_color))
        .replace(
            "{{accent}}",
            &color_or(&theme.accent_color, &defaults.accent_color),
        )
        .replace("{{radius}}", &format!("{}", theme.border_radius.clamp(0.0, 50.0)))
        .replace("{{font_size}}", &theme.font_size.clamp(6, 48).to_string())
}

/// Install (once) and refresh the application stylesheet. Calling it again
/// after a settings change restyles open windows immediately.
pub fn apply(config: &Config) {
    let Some(display) = gdk::Display::default() else {
        return;
    };
    PROVIDER.with(|provider| {
        provider.load_from_string(&build_css(config));
        if !INSTALLED.with(|installed| installed.replace(true)) {
            gtk::style_context_add_provider_for_display(
                &display,
                provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_tokens_are_replaced() {
        let css = build_css(&Config::default());
        assert!(!css.contains("{{"), "unreplaced token in stylesheet");
        assert!(css.contains("background-color: #0a0a0a;"));
        assert!(css.contains("font-size: 16pt;"));
        assert!(css.contains("border-radius: 0px;"));
    }

    #[test]
    fn gtk_css_has_no_important() {
        let rules: String = TEMPLATE
            .split("*/")
            .map(|chunk| chunk.split("/*").next().unwrap_or(""))
            .collect();
        assert!(!rules.contains("!important"));
    }

    #[test]
    fn colors_are_validated() {
        assert!(is_valid_color("#2e2e2e"));
        assert!(is_valid_color("rgba(10, 10, 10, 0.8)"));
        assert!(is_valid_color("red"));
        assert!(!is_valid_color(""));
        assert!(!is_valid_color("not-a-color"));
        assert!(!is_valid_color("red; } window { background: blue"));
    }

    #[test]
    fn invalid_colors_fall_back_to_defaults() {
        let mut config = Config::default();
        config.theme.accent_color = "nope".to_string();
        config.theme.border_radius = 12.0;
        let css = build_css(&config);
        assert!(css.contains("border-color: #4a9eff;"));
        assert!(css.contains("border-radius: 12px;"));
    }
}
