//! The keyboard shortcut that opens Poppi, as a GNOME custom shortcut
//! (Poppi does not grab keys itself). Also finds what else a key
//! combination is already bound to, so the setup can warn about clashes.

use std::process::Command;

const MEDIA_KEYS: &str = "org.gnome.settings-daemon.plugins.media-keys";
const CUSTOM_BASE: &str = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings";
const OUR_NAME: &str = "Poppi Launcher";
/// Schemas holding GNOME's built-in shortcuts.
const BUILT_IN: [&str; 5] = [
    "org.gnome.desktop.wm.keybindings",
    "org.gnome.shell.keybindings",
    "org.gnome.mutter.keybindings",
    "org.gnome.mutter.wayland.keybindings",
    "org.gnome.settings-daemon.plugins.media-keys",
];

fn gsettings(args: &[&str]) -> Result<String, String> {
    let output = Command::new("gsettings")
        .args(args)
        .output()
        .map_err(|e| format!("Could not run gsettings: {}", e))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

/// True on GNOME, where custom shortcuts can be set from here.
pub fn supported() -> bool {
    gsettings(&["get", MEDIA_KEYS, "custom-keybindings"]).is_ok()
}

/// The strings in a GVariant string array as gsettings prints it:
/// `['/a/', '/b/']` or `@as []`.
fn parse_list(value: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut rest = value;
    while let Some(start) = rest.find('\'') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('\'') else {
            break;
        };
        items.push(after[..end].to_string());
        rest = &after[end + 1..];
    }
    items
}

/// A single gsettings string value without its quotes.
fn parse_string(value: &str) -> String {
    value
        .trim()
        .trim_start_matches('\'')
        .trim_end_matches('\'')
        .to_string()
}

/// Accelerators compare equal however GNOME spelled them:
/// `<Primary>Space` and `<Control>space` are the same keys.
fn normalize(accel: &str) -> String {
    accel
        .to_lowercase()
        .replace("<primary>", "<control>")
        .replace("<ctrl>", "<control>")
}

struct Custom {
    path: String,
    name: String,
    command: String,
    binding: String,
}

fn custom_shortcuts() -> Vec<Custom> {
    let Ok(list) = gsettings(&["get", MEDIA_KEYS, "custom-keybindings"]) else {
        return Vec::new();
    };
    parse_list(&list)
        .into_iter()
        .map(|path| {
            let schema = format!("{}.custom-keybinding:{}", MEDIA_KEYS, path);
            let get = |key: &str| {
                gsettings(&["get", &schema, key])
                    .map(|v| parse_string(&v))
                    .unwrap_or_default()
            };
            Custom {
                name: get("name"),
                command: get("command"),
                binding: get("binding"),
                path,
            }
        })
        .collect()
}

/// The key combination currently bound to Poppi, if any.
pub fn current() -> Option<String> {
    custom_shortcuts()
        .into_iter()
        .find(|c| c.name == OUR_NAME)
        .map(|c| c.binding)
        .filter(|b| !b.is_empty())
}

/// What the built-in shortcut lines (`schema key ['<Super>a', …]`) bind
/// `accel` to, as readable names like "switch input source".
fn built_in_uses(listing: &str, accel: &str) -> Vec<String> {
    let wanted = normalize(accel);
    listing
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, ' ');
            let (_schema, key, value) = (parts.next()?, parts.next()?, parts.next()?);
            let values = if value.trim_start().starts_with('[') || value.starts_with("@as") {
                parse_list(value)
            } else {
                vec![parse_string(value)]
            };
            values
                .iter()
                .any(|v| normalize(v) == wanted)
                .then(|| key.replace('-', " "))
        })
        .collect()
}

/// Everything else `accel` is bound to: built-in GNOME shortcuts and other
/// custom shortcuts (by name or command). Poppi's own is not a clash.
pub fn clashes(accel: &str) -> Vec<String> {
    let mut found = Vec::new();
    for schema in BUILT_IN {
        if let Ok(listing) = gsettings(&["list-recursively", schema]) {
            found.extend(built_in_uses(&listing, accel));
        }
    }
    let wanted = normalize(accel);
    for custom in custom_shortcuts() {
        if custom.name != OUR_NAME && normalize(&custom.binding) == wanted {
            found.push(if custom.name.is_empty() {
                custom.command
            } else {
                format!("{} ({})", custom.name, custom.command)
            });
        }
    }
    found
}

/// The first free `customN` slot path, given the paths in use.
fn free_slot(paths: &[String]) -> String {
    (0..)
        .map(|n| format!("{}/custom{}/", CUSTOM_BASE, n))
        .find(|candidate| !paths.contains(candidate))
        .unwrap_or_default()
}

fn variant_list(paths: &[String]) -> String {
    let quoted: Vec<String> = paths.iter().map(|p| format!("'{}'", p)).collect();
    format!("[{}]", quoted.join(", "))
}

/// Bind `accel` to `command`, reusing Poppi's custom shortcut if there is
/// one; other custom shortcuts are left alone.
pub fn bind(accel: &str, command: &str) -> Result<(), String> {
    let shortcuts = custom_shortcuts();
    let path = match shortcuts.iter().find(|c| c.name == OUR_NAME) {
        Some(ours) => ours.path.clone(),
        None => {
            let mut paths: Vec<String> = shortcuts.iter().map(|c| c.path.clone()).collect();
            let path = free_slot(&paths);
            paths.push(path.clone());
            gsettings(&[
                "set",
                MEDIA_KEYS,
                "custom-keybindings",
                &variant_list(&paths),
            ])?;
            path
        }
    };
    let schema = format!("{}.custom-keybinding:{}", MEDIA_KEYS, path);
    gsettings(&["set", &schema, "name", OUR_NAME])?;
    gsettings(&["set", &schema, "command", command])?;
    gsettings(&["set", &schema, "binding", accel])?;
    Ok(())
}

/// What the shortcut should run: the plain name when this binary is the
/// one on `$PATH`, its full path otherwise (e.g. run from a build folder).
pub fn launch_command() -> String {
    let exe = std::env::current_exe().ok();
    let on_path = crate::utils::find_in_path("poppi_launcher");
    match (exe, on_path) {
        (Some(exe), Some(found)) if found.canonicalize().ok() == exe.canonicalize().ok() => {
            "poppi_launcher".to_string()
        }
        (Some(exe), _) => exe.to_string_lossy().into_owned(),
        _ => "poppi_launcher".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gsettings_values_parse() {
        assert_eq!(
            parse_list("['/a/custom0/', '/a/custom1/']"),
            vec!["/a/custom0/", "/a/custom1/"]
        );
        assert!(parse_list("@as []").is_empty());
        assert_eq!(parse_string("'albert toggle'"), "albert toggle");
    }

    #[test]
    fn accelerators_compare_by_meaning() {
        assert_eq!(normalize("<Primary>Space"), normalize("<Control>space"));
        assert_ne!(normalize("<Super>space"), normalize("<Control>space"));
    }

    #[test]
    fn built_in_clashes_are_named() {
        let listing = "\
org.gnome.desktop.wm.keybindings switch-input-source ['<Super>space', 'XF86Keyboard']
org.gnome.desktop.wm.keybindings always-on-top @as []
org.gnome.shell.keybindings toggle-overview ['<Super>s']";
        assert_eq!(
            built_in_uses(listing, "<Super>Space"),
            vec!["switch input source"]
        );
        assert!(built_in_uses(listing, "<Control>space").is_empty());
    }

    #[test]
    fn new_shortcuts_take_the_first_free_slot() {
        let used = vec![
            format!("{}/custom0/", CUSTOM_BASE),
            format!("{}/custom2/", CUSTOM_BASE),
        ];
        assert_eq!(free_slot(&used), format!("{}/custom1/", CUSTOM_BASE));
        assert_eq!(
            variant_list(&used[..1]),
            format!("['{}/custom0/']", CUSTOM_BASE)
        );
    }
}

#[cfg(test)]
mod live_tests {
    /// Reads (never writes) the real GNOME shortcuts:
    /// `cargo test real_clashes -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_clashes() {
        for accel in ["<Control>space", "<Super>space", "<Super>d", "<Control><Alt>p"] {
            println!("{accel}: {:?}", super::clashes(accel));
        }
        println!("current: {:?}", super::current());
        println!("command: {}", super::launch_command());
    }
}
