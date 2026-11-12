//! Lists, activates and closes open windows across GNOME (`window-calls`),
//! Hyprland (`hyprctl`), Sway (`swaymsg`), Niri (`niri`) and X11 (`wmctrl` /
//! `xdotool`).

use crate::matching::Matcher;
use crate::utils;
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
    /// Id from the GNOME window-calls extension.
    Shell(u32),
    /// Hyprland client address, e.g. `0x55a1b2c0`.
    Hyprland(String),
    /// Sway container id.
    Sway(u64),
    /// Niri window id.
    Niri(u64),
    /// X11 window id as printed by wmctrl, e.g. `0x01400004`.
    X11(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct OpenWindow {
    pub id: WindowId,
    pub title: String,
    pub app_name: String,
    /// Raw window class / app_id, used to find the app's icon.
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

#[derive(Deserialize)]
struct HyprClient {
    address: String,
    #[serde(default)]
    mapped: bool,
    #[serde(default)]
    hidden: bool,
    #[serde(default)]
    title: String,
    #[serde(default)]
    class: String,
    #[serde(default)]
    pid: Option<u32>,
    #[serde(default, rename = "focusHistoryID")]
    focus_history_id: i32,
}

#[derive(Deserialize)]
struct SwayWindowProps {
    #[serde(default)]
    class: Option<String>,
}

#[derive(Deserialize)]
struct SwayNode {
    id: u64,
    #[serde(default, rename = "type")]
    node_type: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    app_id: Option<String>,
    #[serde(default)]
    window_properties: Option<SwayWindowProps>,
    #[serde(default)]
    pid: Option<u32>,
    #[serde(default)]
    focused: bool,
    #[serde(default)]
    nodes: Vec<SwayNode>,
    #[serde(default)]
    floating_nodes: Vec<SwayNode>,
}

#[derive(Deserialize)]
struct NiriWindow {
    id: u64,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    app_id: Option<String>,
    #[serde(default)]
    pid: Option<u32>,
    #[serde(default)]
    is_focused: bool,
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

fn parse_hyprland_windows(json: &str, own_pid: u32) -> Result<Vec<OpenWindow>, String> {
    let mut clients: Vec<HyprClient> =
        serde_json::from_str(json).map_err(|e| format!("Unexpected hyprctl output: {}", e))?;
    clients.retain(|c| {
        c.mapped && !c.hidden && c.pid != Some(own_pid) && !is_noise(&c.title, &c.class)
    });
    clients.sort_by_key(|c| c.focus_history_id);
    let mut windows: Vec<OpenWindow> = clients
        .into_iter()
        .map(|c| OpenWindow {
            id: WindowId::Hyprland(c.address),
            app_name: app_name_from_class(&c.class).unwrap_or_else(|| first_word(&c.title)),
            title: c.title,
            class: c.class,
        })
        .collect();
    if windows.len() > 1 {
        windows.rotate_left(1);
    }
    Ok(windows)
}

fn collect_sway_nodes(node: &SwayNode, own_pid: u32, out: &mut Vec<(OpenWindow, bool)>) {
    let is_leaf = node.nodes.is_empty() && node.floating_nodes.is_empty();
    let is_window = matches!(node.node_type.as_str(), "con" | "floating_con")
        && (node.app_id.is_some() || node.window_properties.is_some());
    if is_leaf && is_window && node.pid != Some(own_pid) {
        let title = node.name.clone().unwrap_or_default();
        let class = node
            .app_id
            .clone()
            .or_else(|| {
                node.window_properties
                    .as_ref()
                    .and_then(|p| p.class.clone())
            })
            .unwrap_or_default();
        if !is_noise(&title, &class) {
            out.push((
                OpenWindow {
                    id: WindowId::Sway(node.id),
                    app_name: app_name_from_class(&class).unwrap_or_else(|| first_word(&title)),
                    title,
                    class,
                },
                node.focused,
            ));
        }
    }
    for child in node.nodes.iter().chain(&node.floating_nodes) {
        collect_sway_nodes(child, own_pid, out);
    }
}

fn parse_sway_tree(json: &str, own_pid: u32) -> Result<Vec<OpenWindow>, String> {
    let root: SwayNode =
        serde_json::from_str(json).map_err(|e| format!("Unexpected swaymsg output: {}", e))?;
    let mut pairs = Vec::new();
    collect_sway_nodes(&root, own_pid, &mut pairs);
    // Keep the currently focused window last, like Alt+Tab.
    pairs.sort_by_key(|(_, focused)| *focused);
    Ok(pairs.into_iter().map(|(w, _)| w).collect())
}

fn parse_niri_windows(json: &str, own_pid: u32) -> Result<Vec<OpenWindow>, String> {
    let raw: Vec<NiriWindow> =
        serde_json::from_str(json).map_err(|e| format!("Unexpected niri output: {}", e))?;
    let mut pairs: Vec<(OpenWindow, bool)> = raw
        .into_iter()
        .filter(|w| w.pid != Some(own_pid))
        .filter_map(|w| {
            let title = w.title?;
            let class = w.app_id.unwrap_or_default();
            if is_noise(&title, &class) {
                return None;
            }
            Some((
                OpenWindow {
                    id: WindowId::Niri(w.id),
                    app_name: app_name_from_class(&class).unwrap_or_else(|| first_word(&title)),
                    title,
                    class,
                },
                w.is_focused,
            ))
        })
        .collect();
    pairs.sort_by_key(|(_, focused)| *focused);
    Ok(pairs.into_iter().map(|(w, _)| w).collect())
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
    let mut windows = parse_shell_windows(&json, std::process::id())?;
    // The extension lists windows in stacking order, bottom first. Reversed,
    // that is most recently used first; the top one is where the user just
    // was, so it goes last, like Alt+Tab.
    windows.reverse();
    if windows.len() > 1 {
        windows.rotate_left(1);
    }
    Ok(windows)
}

fn list_hyprland_windows() -> Result<Vec<OpenWindow>, String> {
    let output = Command::new("hyprctl")
        .args(["clients", "-j"])
        .output()
        .map_err(|e| format!("hyprctl failed: {}", e))?;
    if !output.status.success() {
        return Err("hyprctl clients failed".to_string());
    }
    parse_hyprland_windows(&String::from_utf8_lossy(&output.stdout), std::process::id())
}

fn list_sway_windows() -> Result<Vec<OpenWindow>, String> {
    let output = Command::new("swaymsg")
        .args(["-t", "get_tree"])
        .output()
        .map_err(|e| format!("swaymsg failed: {}", e))?;
    if !output.status.success() {
        return Err("swaymsg get_tree failed".to_string());
    }
    parse_sway_tree(&String::from_utf8_lossy(&output.stdout), std::process::id())
}

fn list_niri_windows() -> Result<Vec<OpenWindow>, String> {
    let output = Command::new("niri")
        .args(["msg", "-j", "windows"])
        .output()
        .map_err(|e| format!("niri failed: {}", e))?;
    if !output.status.success() {
        return Err("niri msg windows failed".to_string());
    }
    parse_niri_windows(&String::from_utf8_lossy(&output.stdout), std::process::id())
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
        let output = Command::new("xdotool")
            .args([subcommand, id])
            .output()
            .ok()?;
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
    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() && utils::has_command("hyprctl") {
        if let Ok(windows) = list_hyprland_windows() {
            return Ok(windows);
        }
    }
    if std::env::var_os("SWAYSOCK").is_some() && utils::has_command("swaymsg") {
        if let Ok(windows) = list_sway_windows() {
            return Ok(windows);
        }
    }
    if std::env::var_os("NIRI_SOCKET").is_some() && utils::has_command("niri") {
        if let Ok(windows) = list_niri_windows() {
            return Ok(windows);
        }
    }
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
            "No windows found. On GNOME install the window-calls extension; Hyprland, Sway and Niri work automatically."
                .to_string(),
        );
    }
    Err(errors
        .into_iter()
        .next()
        .unwrap_or_else(|| "No windows found. Install wmctrl to list windows.".to_string()))
}

pub fn search<'a>(
    matcher: &Matcher,
    query: &str,
    windows: &'a [OpenWindow],
) -> Vec<&'a OpenWindow> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        // Keep the order from `list_windows` (most recently used first).
        return windows.iter().collect();
    }

    let mut results: Vec<(&OpenWindow, i64)> = windows
        .iter()
        .filter_map(|window| {
            let title = matcher.score(&query, &window.title.to_lowercase());
            // A title match outranks an equally good app-name match.
            let app = matcher
                .score(&query, &window.app_name.to_lowercase())
                .map(|s| s - 60);
            title
                .into_iter()
                .chain(app)
                .max()
                .map(|score| (window, score))
        })
        .collect();
    results.sort_by_key(|item| std::cmp::Reverse(item.1));
    results.into_iter().map(|(window, _)| window).collect()
}

fn run_ok(command: &mut Command) -> bool {
    command
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn activate(window: &OpenWindow) -> Result<(), String> {
    match &window.id {
        WindowId::Shell(id) => shell_call("Activate", Some(&(*id,).to_variant())).map(|_| ()),
        WindowId::Hyprland(addr) => {
            if run_ok(Command::new("hyprctl").args([
                "dispatch",
                "focuswindow",
                &format!("address:{}", addr),
            ])) {
                Ok(())
            } else {
                Err(format!("Could not switch to window: {}", window.title))
            }
        }
        WindowId::Sway(id) => {
            if run_ok(Command::new("swaymsg").arg(format!("[con_id={}] focus", id))) {
                Ok(())
            } else {
                Err(format!("Could not switch to window: {}", window.title))
            }
        }
        WindowId::Niri(id) => {
            if run_ok(Command::new("niri").args([
                "msg",
                "action",
                "focus-window",
                "--id",
                &id.to_string(),
            ])) {
                Ok(())
            } else {
                Err(format!("Could not switch to window: {}", window.title))
            }
        }
        WindowId::X11(hex) => {
            if utils::has_command("wmctrl")
                && run_ok(Command::new("wmctrl").args(["-i", "-a", hex]))
            {
                return Ok(());
            }
            if utils::has_command("xdotool") {
                let decimal = u64::from_str_radix(hex.trim_start_matches("0x"), 16)
                    .map_err(|_| "Invalid window id".to_string())?;
                if run_ok(Command::new("xdotool").args(["windowactivate", &decimal.to_string()])) {
                    return Ok(());
                }
            }
            Err(format!("Could not switch to window: {}", window.title))
        }
    }
}

pub fn close(window: &OpenWindow) -> Result<(), String> {
    match &window.id {
        WindowId::Shell(id) => shell_call("Close", Some(&(*id,).to_variant())).map(|_| ()),
        WindowId::Hyprland(addr) => {
            if run_ok(Command::new("hyprctl").args([
                "dispatch",
                "closewindow",
                &format!("address:{}", addr),
            ])) {
                Ok(())
            } else {
                Err(format!("Could not close window: {}", window.title))
            }
        }
        WindowId::Sway(id) => {
            if run_ok(Command::new("swaymsg").arg(format!("[con_id={}] kill", id))) {
                Ok(())
            } else {
                Err(format!("Could not close window: {}", window.title))
            }
        }
        WindowId::Niri(id) => {
            if run_ok(Command::new("niri").args([
                "msg",
                "action",
                "close-window",
                "--id",
                &id.to_string(),
            ])) {
                Ok(())
            } else {
                Err(format!("Could not close window: {}", window.title))
            }
        }
        WindowId::X11(hex) => {
            if utils::has_command("wmctrl")
                && run_ok(Command::new("wmctrl").args(["-i", "-c", hex]))
            {
                return Ok(());
            }
            if utils::has_command("xdotool") {
                let decimal = u64::from_str_radix(hex.trim_start_matches("0x"), 16)
                    .map_err(|_| "Invalid window id".to_string())?;
                if run_ok(Command::new("xdotool").args(["windowclose", &decimal.to_string()])) {
                    return Ok(());
                }
            }
            Err(format!("Could not close window: {}", window.title))
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
        assert_eq!(
            app_name_from_class("app.zen_browser.zen").as_deref(),
            Some("Zen")
        );
        assert_eq!(
            app_name_from_class("cursor.Cursor").as_deref(),
            Some("Cursor")
        );
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
        assert_eq!(windows[1].title, "Notification settings");
        assert_eq!(windows[1].app_name, "Notification");
        assert_eq!(windows[1].id, WindowId::Shell(3118351006));
    }

    #[test]
    fn hyprland_sway_and_niri_json_parsers() {
        let hypr = r#"[
            {"address":"0x1","mapped":true,"hidden":false,"title":"Active","class":"kitty","pid":10,"focusHistoryID":0},
            {"address":"0x2","mapped":true,"hidden":false,"title":"Docs","class":"org.mozilla.firefox","pid":11,"focusHistoryID":1},
            {"address":"0x3","mapped":false,"hidden":false,"title":"Ghost","class":"x","pid":12,"focusHistoryID":2}
        ]"#;
        let hw = parse_hyprland_windows(hypr, 99).unwrap();
        assert_eq!(hw.len(), 2);
        // Previous window comes first, current last (Alt+Tab order).
        assert_eq!(hw[0].title, "Docs");
        assert_eq!(hw[0].app_name, "Firefox");
        assert_eq!(hw[1].title, "Active");

        let sway = r#"{
            "id": 1, "type": "root",
            "nodes": [
                {"id": 10, "type": "con", "name": "Focused", "app_id": "kitty", "pid": 20, "focused": true},
                {"id": 11, "type": "con", "name": "Editor", "window_properties": {"class": "dev.zed.Zed"}, "pid": 21, "focused": false}
            ]
        }"#;
        let sw = parse_sway_tree(sway, 99).unwrap();
        assert_eq!(sw.len(), 2);
        assert_eq!(sw[0].title, "Editor");
        assert_eq!(sw[0].app_name, "Zed");
        assert_eq!(sw[1].title, "Focused");

        let niri = r#"[
            {"id": 7, "title": "Current", "app_id": "kitty", "pid": 30, "is_focused": true},
            {"id": 8, "title": "Browser", "app_id": "firefox", "pid": 31, "is_focused": false}
        ]"#;
        let nw = parse_niri_windows(niri, 99).unwrap();
        assert_eq!(nw.len(), 2);
        assert_eq!(nw[0].title, "Browser");
        assert_eq!(nw[1].title, "Current");
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
    fn search_ranks_title_over_app_and_keeps_order_for_empty_query() {
        let matcher = Matcher::new();
        let windows = vec![
            window("zsh", "Kitty"),
            window("Kitty docs", "Firefox"),
            window("Alpha", "Files"),
        ];

        let all = search(&matcher, "", &windows);
        assert_eq!(all[0].title, "zsh");
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
