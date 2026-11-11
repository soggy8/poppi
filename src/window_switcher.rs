//! Lists and activates open windows.
//!
//! On GNOME the `window-calls` shell extension is the only way to see native
//! Wayland windows, and it reports XWayland windows too. Without it we fall
//! back to `wmctrl`/`xdotool`, which only see X11 and XWayland windows.

use crate::utils;
use crate::matching::Matcher;
use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use serde::Deserialize;
use std::process::Command;

const SHELL_BUS: &str = "org.gnome.Shell";
const WINDOWS_PATH: &str = "/org/gnome/Shell/Extensions/Windows";
const WINDOWS_IFACE: &str = "org.gnome.Shell.Extensions.Windows";

#[derive(Debug, Clone, PartialEq)]
pub enum WindowId {
    /// Id from the window-calls extension.
    Shell(u32),
    /// X11 window id as printed by wmctrl, e.g. `0x01400004`.
    X11(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct OpenWindow {
    pub id: WindowId,
    pub title: String,
    pub app_name: String,
    /// Raw window class, used to find the app's icon.
    pub class: String,
}

#[derive(Deserialize)]
struct ShellWindow {
    id: u32,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    wm_class: Option<String>,
    #[serde(default)]
    window_type: u32,
    #[serde(default)]
    pid: Option<u32>,
}

/// Readable app name from a window class: `dev.zed.Zed` -> `Zed`,
/// `cursor.Cursor` -> `Cursor`, `kitty` -> `Kitty`.
fn app_name_from_class(class: &str) -> Option<String> {
    let segment = class.rsplit('.').find(|s| !s.is_empty())?;
    Some(utils::capitalize(segment))
}

fn is_noise(title: &str, class: &str) -> bool {
    title.trim().is_empty() || class.to_lowercase().contains("xwaylandvideobridge")
}

fn first_word(title: &str) -> String {
    title
        .split_whitespace()
        .next()
        .unwrap_or("Window")
        .to_string()
}

fn parse_shell_windows(json: &str, own_pid: u32) -> Result<Vec<OpenWindow>, String> {
    let windows: Vec<ShellWindow> =
        serde_json::from_str(json).map_err(|e| format!("Unexpected window list: {}", e))?;
    Ok(windows
        .into_iter()
        // 0 is META_WINDOW_NORMAL; skip docks, menus, tooltips and ourselves.
        .filter(|w| w.window_type == 0 && w.pid != Some(own_pid))
        .filter_map(|w| {
            let title = w.title?;
            let class = w.wm_class.unwrap_or_default();
            if is_noise(&title, &class) {
                return None;
            }
            Some(OpenWindow {
                id: WindowId::Shell(w.id),
                app_name: app_name_from_class(&class).unwrap_or_else(|| first_word(&title)),
                title,
                class,
            })
        })
        .collect())
}

/// Parse `wmctrl -l -x` output: `ID DESKTOP CLASS HOST TITLE...`.
fn parse_wmctrl(output: &str) -> Vec<(String, String, String)> {
    output
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let id = parts.next()?.to_string();
            let _desktop = parts.next()?;
            let class = parts.next()?.to_string();
            let _host = parts.next()?;
            let title = parts.collect::<Vec<_>>().join(" ");
            (!is_noise(&title, &class)).then_some((id, class, title))
        })
        .collect()
}

fn shell_call(method: &str, parameters: Option<&glib::Variant>) -> Result<glib::Variant, String> {
    let connection = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE)
        .map_err(|e| e.to_string())?;
    connection
        .call_sync(
            Some(SHELL_BUS),
            WINDOWS_PATH,
            WINDOWS_IFACE,
            method,
            parameters,
            None,
            gio::DBusCallFlags::NONE,
            1500,
            gio::Cancellable::NONE,
        )
        .map_err(|e| e.to_string())
}

fn list_shell_windows() -> Result<Vec<OpenWindow>, String> {
    let reply = shell_call("List", None)?;
    let json = reply
        .try_child_value(0)
        .and_then(|v| v.str().map(str::to_string))
        .ok_or_else(|| "Unexpected reply from window-calls".to_string())?;
    parse_shell_windows(&json, std::process::id())
}

/// DOCK, DESKTOP, SPLASH and friends are not windows a user switches to.
fn is_normal_x11_window(window_id: &str) -> bool {
    let output = Command::new("xprop")
        .args(["-id", window_id, "_NET_WM_WINDOW_TYPE"])
        .output();
    match output {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            // A window without the property is a normal window.
            !stdout.contains("_NET_WM_WINDOW_TYPE_")
                || stdout.contains("_NET_WM_WINDOW_TYPE_NORMAL")
                || stdout.contains("_NET_WM_WINDOW_TYPE_DIALOG")
        }
        _ => true,
    }
}

fn list_wmctrl_windows() -> Result<Vec<OpenWindow>, String> {
    let output = Command::new("wmctrl")
        .args(["-l", "-x"])
        .output()
        .map_err(|e| format!("wmctrl failed: {}", e))?;
    if !output.status.success() {
        return Err(format!(
            "wmctrl failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let has_xprop = utils::has_command("xprop");
    Ok(parse_wmctrl(&String::from_utf8_lossy(&output.stdout))
        .into_iter()
        .take(100)
        .filter(|(id, _, _)| !has_xprop || is_normal_x11_window(id))
        .map(|(id, class, title)| OpenWindow {
            id: WindowId::X11(id),
            app_name: app_name_from_class(&class).unwrap_or_else(|| first_word(&title)),
            title,
            class,
        })
        .collect())
}

fn list_xdotool_windows() -> Result<Vec<OpenWindow>, String> {
    let output = Command::new("xdotool")
        .args(["search", "--onlyvisible", "--name", "."])
        .output()
        .map_err(|e| format!("xdotool failed: {}", e))?;
    if !output.status.success() {
        return Err("xdotool search failed".to_string());
    }

    let query = |subcommand: &str, id: &str| -> Option<String> {
        let output = Command::new("xdotool").args([subcommand, id]).output().ok()?;
        output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
    };

    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .take(50)
        .filter_map(|decimal| {
            let title = query("getwindowname", decimal)?;
            let class = query("getwindowclassname", decimal).unwrap_or_default();
            if is_noise(&title, &class) {
                return None;
            }
            let id: u64 = decimal.parse().ok()?;
            Some(OpenWindow {
                id: WindowId::X11(format!("0x{:x}", id)),
                app_name: app_name_from_class(&class).unwrap_or_else(|| first_word(&title)),
                title,
                class,
            })
        })
        .collect())
}

/// Blocking; spawns helper processes. Call it off the main thread.
pub fn list_windows() -> Result<Vec<OpenWindow>, String> {
    if let Ok(windows) = list_shell_windows() {
        return Ok(windows);
    }

    let mut errors = Vec::new();
    type Lister = fn() -> Result<Vec<OpenWindow>, String>;
    let fallbacks: [(&str, Lister); 2] = [
        ("wmctrl", list_wmctrl_windows),
        ("xdotool", list_xdotool_windows),
    ];
    for (tool, list) in fallbacks {
        if !utils::has_command(tool) {
            continue;
        }
        match list() {
            Ok(windows) if !windows.is_empty() => return Ok(windows),
            Ok(_) => {}
            Err(e) => errors.push(e),
        }
    }

    // On Wayland the X11 tools cannot see native windows, so their errors
    // are less useful than pointing at the extension.
    if utils::is_wayland() {
        return Err(
            "No windows found. Install the window-calls GNOME extension to list Wayland windows."
                .to_string(),
        );
    }
    Err(errors
        .into_iter()
        .next()
        .unwrap_or_else(|| "No windows found. Install wmctrl to list windows.".to_string()))
}

pub fn search<'a>(matcher: &Matcher, query: &str, windows: &'a [OpenWindow]) -> Vec<&'a OpenWindow> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        let mut all: Vec<&OpenWindow> = windows.iter().collect();
        all.sort_by_key(|w| w.title.to_lowercase());
        return all;
    }

    let mut results: Vec<(&OpenWindow, i64)> = windows
        .iter()
        .filter_map(|window| {
            let title = matcher.score(&query, &window.title.to_lowercase());
            // A title match outranks an equally good app-name match.
            let app = matcher
                .score(&query, &window.app_name.to_lowercase())
                .map(|s| s - 60);
            title.into_iter().chain(app).max().map(|score| (window, score))
        })
        .collect();
    results.sort_by(|a, b| b.1.cmp(&a.1));
    results.into_iter().map(|(window, _)| window).collect()
}

pub fn activate(window: &OpenWindow) -> Result<(), String> {
    match &window.id {
        WindowId::Shell(id) => shell_call("Activate", Some(&(*id,).to_variant())).map(|_| ()),
        WindowId::X11(hex) => {
            let ran = |command: &mut Command| {
                command
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false)
            };
            if utils::has_command("wmctrl") && ran(Command::new("wmctrl").args(["-i", "-a", hex])) {
                return Ok(());
            }
            if utils::has_command("xdotool") {
                let decimal = u64::from_str_radix(hex.trim_start_matches("0x"), 16)
                    .map_err(|_| "Invalid window id".to_string())?;
                if ran(Command::new("xdotool").args(["windowactivate", &decimal.to_string()])) {
                    return Ok(());
                }
            }
            Err(format!("Could not switch to window: {}", window.title))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(title: &str, app: &str) -> OpenWindow {
        OpenWindow {
            id: WindowId::Shell(1),
            title: title.to_string(),
            app_name: app.to_string(),
            class: app.to_lowercase(),
        }
    }

    #[test]
    fn app_names_from_classes() {
        assert_eq!(app_name_from_class("dev.zed.Zed").as_deref(), Some("Zed"));
        assert_eq!(app_name_from_class("app.zen_browser.zen").as_deref(), Some("Zen"));
        assert_eq!(app_name_from_class("cursor.Cursor").as_deref(), Some("Cursor"));
        assert_eq!(app_name_from_class("kitty").as_deref(), Some("Kitty"));
        assert_eq!(app_name_from_class("über").as_deref(), Some("Über"));
        assert_eq!(app_name_from_class(""), None);
    }

    #[test]
    fn shell_json_is_filtered() {
        let json = r#"[
            {"wm_class":"kitty","title":"It's a \"shell\"","pid":10,"id":1,"window_type":0},
            {"wm_class":"gnome-shell","title":"panel","pid":11,"id":2,"window_type":2},
            {"wm_class":"poppi_launcher","title":"Poppi Launcher","pid":99,"id":3,"window_type":0},
            {"wm_class":"dev.zed.Zed","title":"","pid":12,"id":4,"window_type":0},
            {"wm_class":null,"title":"Notification settings","pid":13,"id":3118351006,"window_type":0}
        ]"#;
        let windows = parse_shell_windows(json, 99).unwrap();
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].title, "It's a \"shell\"");
        assert_eq!(windows[0].app_name, "Kitty");
        assert_eq!(windows[0].id, WindowId::Shell(1));
        // A normal window whose title merely contains "notification" is kept.
        assert_eq!(windows[1].title, "Notification settings");
        assert_eq!(windows[1].app_name, "Notification");
        assert_eq!(windows[1].id, WindowId::Shell(3118351006));
    }

    /// Manual check against the running desktop:
    /// `cargo test -- --ignored --nocapture lists_windows`
    #[test]
    #[ignore = "needs a desktop session"]
    fn lists_windows_on_this_desktop() {
        let windows = list_windows().expect("window listing failed");
        for window in &windows {
            println!("{:?}  {}  [{}]", window.id, window.title, window.app_name);
        }
        assert!(!windows.is_empty());
    }

    #[test]
    fn shell_json_errors_are_reported() {
        assert!(parse_shell_windows("not json", 1).is_err());
    }

    #[test]
    fn wmctrl_lines_drop_the_host_column() {
        let output = "0x01400004  0 cursor.Cursor         fedora main.rs - poppi - Cursor\n\
                      0x01400009 -1 a.B fedora \n\
                      garbage\n";
        let parsed = parse_wmctrl(output);
        assert_eq!(
            parsed,
            vec![(
                "0x01400004".to_string(),
                "cursor.Cursor".to_string(),
                "main.rs - poppi - Cursor".to_string()
            )]
        );
    }

    #[test]
    fn search_ranks_title_over_app_and_sorts_empty_query() {
        let matcher = Matcher::new();
        let windows = vec![
            window("zsh", "Kitty"),
            window("Kitty docs", "Firefox"),
            window("Alpha", "Files"),
        ];

        let all = search(&matcher, "", &windows);
        assert_eq!(all[0].title, "Alpha");
        assert_eq!(all.len(), 3);

        // Equally good matches: the title wins over the app name.
        let kit = search(&matcher, "kit", &windows);
        assert_eq!(kit.len(), 2);
        assert_eq!(kit[0].title, "Kitty docs");

        // Typing the exact app name puts that app's windows first.
        let kitty = search(&matcher, "kitty", &windows);
        assert_eq!(kitty[0].app_name, "Kitty");

        assert!(search(&matcher, "qqqq", &windows).is_empty());
    }
}
