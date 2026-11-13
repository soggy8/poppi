use crate::config::{Config, ThemeConfig};
use gtk::prelude::*;
use gtk::{gdk, gio};

const TEMPLATE: &str = include_str!("style.css");
const INTERFACE_SCHEMA: &str = "org.gnome.desktop.interface";

thread_local! {
    static PROVIDER: gtk::CssProvider = gtk::CssProvider::new();
    static INSTALLED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Kept alive so its `changed` signal keeps firing.
    static INTERFACE_SETTINGS: std::cell::RefCell<Option<gio::Settings>> =
        const { std::cell::RefCell::new(None) };
}

pub fn is_valid_color(color: &str) -> bool {
    let color = color.trim();
    // Reject anything that could break out of the declaration it is pasted into.
    !color.is_empty() && !color.contains([';', '{', '}']) && gdk::RGBA::parse(color).is_ok()
}

fn color_or(color: &str, fallback: &str) -> String {
    if is_valid_color(color) {
        color.trim().to_string()
    } else {
        eprintln!("Ignoring invalid color {:?}, using {}", color, fallback);
        fallback.to_string()
    }
}

/// `#rrggbb` for a colour GTK can parse, for use in Pango markup.
pub fn to_hex(color: &str) -> Option<String> {
    let rgba = gdk::RGBA::parse(color.trim()).ok()?;
    let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    Some(format!(
        "#{:02x}{:02x}{:02x}",
        channel(rgba.red()),
        channel(rgba.green()),
        channel(rgba.blue())
    ))
}

fn interface_settings() -> Option<gio::Settings> {
    INTERFACE_SETTINGS.with(|slot| {
        if slot.borrow().is_none() {
            // Creating a Settings object for a missing schema aborts.
            let exists = gio::SettingsSchemaSource::default()
                .and_then(|source| source.lookup(INTERFACE_SCHEMA, true))
                .is_some();
            if exists {
                *slot.borrow_mut() = Some(gio::Settings::new(INTERFACE_SCHEMA));
            }
        }
        slot.borrow().clone()
    })
}

fn system_prefers_dark() -> bool {
    interface_settings()
        .map(|settings| settings.string("color-scheme") == "prefer-dark")
        .unwrap_or(true)
}

/// Call `callback` whenever the desktop switches between light and dark.
pub fn watch_system_scheme(callback: impl Fn() + 'static) {
    if let Some(settings) = interface_settings() {
        settings.connect_changed(Some("color-scheme"), move |_, _| callback());
    }
}

pub fn is_dark(config: &Config) -> bool {
    match config.theme.mode.as_str() {
        "light" => false,
        "system" => system_prefers_dark(),
        _ => true,
    }
}

/// The three colours everything else is derived from.
pub struct Palette {
    pub background: String,
    pub text: String,
    pub accent: String,
}

pub fn palette(config: &Config, dark: bool) -> Palette {
    let defaults = ThemeConfig::default();
    let theme = &config.theme;
    let (background, text) = if dark {
        (
            color_or(&theme.background_color, &defaults.background_color),
            color_or(&theme.text_color, &defaults.text_color),
        )
    } else {
        (
            color_or(
                &theme.light_background_color,
                &defaults.light_background_color,
            ),
            color_or(&theme.light_text_color, &defaults.light_text_color),
        )
    };
    Palette {
        background,
        text,
        accent: color_or(&theme.accent_color, &defaults.accent_color),
    }
}

/// The accent colour as `#rrggbb`, for emphasising matched characters.
pub fn accent_hex(config: &Config) -> String {
    let palette = palette(config, is_dark(config));
    to_hex(&palette.accent).unwrap_or_else(|| "#4a9eff".to_string())
}

pub fn build_css(config: &Config, dark: bool) -> String {
    let palette = palette(config, dark);
    let radius = config.theme.border_radius.clamp(0.0, 50.0);
    let font_size = config.theme.font_size.clamp(6, 48);
    TEMPLATE
        .replace("{{background}}", &palette.background)
        .replace("{{text}}", &palette.text)
        .replace("{{accent}}", &palette.accent)
        .replace("{{error}}", if dark { "#ff8f8f" } else { "#c62828" })
        .replace("{{shadow}}", if dark { "0.55" } else { "0.22" })
        .replace("{{radius}}", &format!("{}", radius))
        // Inner elements never look rounder than the card itself.
        .replace(
            "{{radius_small}}",
            &format!("{:.1}", (radius * 0.6).min(9.0)),
        )
        .replace("{{font_size}}", &font_size.to_string())
        .replace(
            "{{title_size}}",
            &format!("{:.1}", (f64::from(font_size) * 0.8).max(9.0)),
        )
        .replace(
            "{{result_size}}",
            &format!("{:.1}", f64::from(font_size) * 1.15),
        )
}

/// Install (once) and refresh the application stylesheet. Calling it again
/// after a settings change restyles open windows immediately.
pub fn apply(config: &Config) {
    let Some(display) = gdk::Display::default() else {
        return;
    };
    let dark = is_dark(config);
    // Stock widgets (the settings window, popovers) follow the same mode.
    if let Some(settings) = gtk::Settings::default() {
        settings.set_gtk_application_prefer_dark_theme(dark);
    }
    PROVIDER.with(|provider| {
        provider.load_from_string(&build_css(config, dark));
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
    fn all_tokens_are_replaced_in_both_modes() {
        for dark in [true, false] {
            let css = build_css(&Config::default(), dark);
            assert!(!css.contains("{{"), "unreplaced token in stylesheet");
            assert!(css.contains("font-size: 16pt;"));
            assert!(css.contains("border-radius: 14px;"));
        }
        assert!(build_css(&Config::default(), true).contains("background-color: #161618;"));
        assert!(build_css(&Config::default(), false).contains("background-color: #f7f7f8;"));
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
        config.theme.border_radius = 4.0;
        let css = build_css(&config, true);
        assert!(css.contains("caret-color: #4a9eff;"));
        assert!(css.contains("border-radius: 4px;"));
        assert!(css.contains("border-radius: 2.4px;"));
        assert_eq!(accent_hex(&config), "#4a9eff");
    }

    #[test]
    fn mode_selects_the_palette() {
        let mut config = Config::default();
        assert!(is_dark(&config));
        config.theme.mode = "light".to_string();
        assert!(!is_dark(&config));
        assert_eq!(palette(&config, false).text, "#1c1c1e");
    }

    #[test]
    fn hex_conversion() {
        assert_eq!(to_hex("rgb(255, 0, 128)").as_deref(), Some("#ff0080"));
        assert_eq!(to_hex("#5294e2").as_deref(), Some("#5294e2"));
        assert_eq!(to_hex("nope"), None);
    }
}
