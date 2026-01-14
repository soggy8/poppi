use std::process::Command;
use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;

#[derive(Debug, Clone)]
pub struct OpenWindow {
    pub window_id: String,
    pub title: String,
    pub app_name: String,
}

pub struct WindowSwitcher {
    matcher: SkimMatcherV2,
}

impl WindowSwitcher {
    pub fn new() -> Self {
        Self {
            matcher: SkimMatcherV2::default(),
        }
    }

    pub fn get_open_windows(&self) -> Result<Vec<OpenWindow>, Box<dyn std::error::Error>> {
        // For GNOME, use wmctrl which works reliably for XWayland windows
        // NOTE: Native Wayland windows (like Kitty, native GTK4 apps) won't appear
        // without a GNOME Shell extension. This is a Wayland security limitation.
        if Self::has_command("wmctrl") {
            match Self::get_windows_wmctrl_filtered() {
                Ok(windows) if !windows.is_empty() => return Ok(windows),
                Ok(_) => {}, // Empty result, try fallback
                Err(e) => eprintln!("wmctrl failed: {}", e), // Log but continue
            }
        }
        
        // Fallback: try xdotool (but filter heavily)
        if Self::has_command("xdotool") {
            match Self::get_windows_xdotool_filtered() {
                Ok(windows) if !windows.is_empty() => return Ok(windows),
                Ok(_) => {}, // Empty result
                Err(e) => eprintln!("xdotool failed: {}", e), // Log but continue
            }
        }
        
        Err("No windows found. Note: Native Wayland windows require a GNOME Shell extension.".into())
    }
    
    fn should_include_window(title: &str, app_name: &str) -> bool {
        // Filter out system windows, popups, and unwanted windows
        let title_lower = title.to_lowercase();
        let app_lower = app_name.to_lowercase();
        
        // Skip empty titles
        if title.trim().is_empty() {
            return false;
        }
        
        // Skip system windows and the launcher itself
        let skip_patterns = [
            "wayland to x recording bridge",
            "xwayland video bridge",
            "xwaylandvideobridge",
            "desktop window",
            "gnome-shell",
            "mutter",
            "notification",
            "popup",
            "tooltip",
            "dropdown",
            "menu",
            "poppi_launcher", // Don't show the launcher itself
        ];
        
        for pattern in &skip_patterns {
            if title_lower.contains(pattern) || app_lower.contains(pattern) {
                return false;
            }
        }
        
        // Skip windows with very short titles (likely system windows)
        if title.trim().len() < 3 {
            return false;
        }
        
        true
    }

    fn get_windows_wmctrl_filtered() -> Result<Vec<OpenWindow>, Box<dyn std::error::Error>> {
        // Try without timeout first (faster), fallback to timeout if needed
        let output = match Command::new("wmctrl")
            .arg("-l")
            .arg("-x")  // Show window class names too
            .output()
        {
            Ok(o) => o,
            Err(_) => {
                // Fallback with timeout
                Command::new("timeout")
                    .arg("1")
                    .arg("wmctrl")
                    .arg("-l")
                    .arg("-x")
                    .output()?
            }
        };
        
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("wmctrl failed: {}", stderr).into());
        }
        
        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut windows = Vec::new();
        
        for line in stdout.lines().take(100) { // Limit to 100 windows
            // wmctrl -l -x format: ID DESKTOP WM_CLASS TITLE
            // Example: 0x01400004  0 cursor.Cursor         fedora window_switcher.rs - poppi_launcher - Cursor
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 4 {
                let window_id = parts[0].to_string();
                let wm_class = parts[2]; // WM_CLASS is at index 2
                
                // Title is everything from index 3 onwards (may contain spaces)
                let title = parts[3..].join(" ");
                
                // Skip empty titles
                if title.trim().is_empty() {
                    continue;
                }
                
                // Extract app name from WM_CLASS (format: AppName.instance)
                let app_name = if !wm_class.is_empty() && !wm_class.contains("xwaylandvideobridge") {
                    // WM_CLASS format is usually "AppName.instance" - take first part and make it readable
                    let class_name = wm_class.split('.').next().unwrap_or(wm_class);
                    // Convert to title case (e.g., "cursor" -> "Cursor", "kitty" -> "Kitty")
                    if let Some(first_char) = class_name.chars().next() {
                        format!("{}{}", first_char.to_uppercase(), &class_name[1..])
                    } else {
                        class_name.to_string()
                    }
                } else if let Some(first_word) = title.split_whitespace().next() {
                    first_word.to_string()
                } else {
                    title.clone()
                };
                
                // Filter windows
                if Self::should_include_window(&title, &app_name) {
                    windows.push(OpenWindow {
                        window_id,
                        title,
                        app_name,
                    });
                }
            }
        }
        
        Ok(windows)
    }
    
    fn get_windows_wmctrl() -> Result<Vec<OpenWindow>, Box<dyn std::error::Error>> {
        Self::get_windows_wmctrl_filtered()
    }

    fn get_windows_xdotool_filtered() -> Result<Vec<OpenWindow>, Box<dyn std::error::Error>> {
        // Get all windows (not just visible) to catch more windows
        let output = Command::new("xdotool")
            .arg("search")
            .arg("--class")
            .arg("")
            .output()?;
        
        if !output.status.success() {
            return Err("xdotool search failed".into());
        }
        
        let window_ids: Vec<String> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .take(100) // Limit to 100 windows
            .collect();
        
        if window_ids.is_empty() {
            return Ok(Vec::new());
        }
        
        // Batch get window names
        let mut windows = Vec::new();
        
        for window_id in window_ids {
            // Get window name
            let name_output = Command::new("xdotool")
                .arg("getwindowname")
                .arg(&window_id)
                .output();
            
            if let Ok(output) = name_output {
                if output.status.success() {
                    let title = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    
                    // Skip empty or invalid titles
                    if !title.is_empty() && title != "N/A" && title.len() > 1 {
                        // Get window class for app name
                        let app_name = Command::new("xdotool")
                            .arg("getwindowclassname")
                            .arg(&window_id)
                            .output()
                            .ok()
                            .and_then(|o| {
                                if o.status.success() {
                                    let name = String::from_utf8_lossy(&o.stdout).trim().to_string();
                                    if !name.is_empty() {
                                        Some(name.split('.').next().unwrap_or(&name).to_string())
                                    } else {
                                        None
                                    }
                                } else {
                                    None
                                }
                            })
                            .unwrap_or_else(|| {
                                title.split_whitespace().next().unwrap_or("Window").to_string()
                            });
                        
                        // Filter windows
                        if Self::should_include_window(&title, &app_name) {
                            windows.push(OpenWindow {
                                window_id,
                                title,
                                app_name,
                            });
                        }
                    }
                }
            }
        }
        
        Ok(windows)
    }
    
    fn get_windows_xdotool() -> Result<Vec<OpenWindow>, Box<dyn std::error::Error>> {
        Self::get_windows_xdotool_filtered()
    }

    fn get_windows_gnome_wayland() -> Result<Vec<OpenWindow>, Box<dyn std::error::Error>> {
        // NOTE: GNOME Shell's Eval method requires extensions to be enabled
        // and may not work in all GNOME versions. For native Wayland windows,
        // users may need to install the "window-calls" extension.
        // 
        // For now, we return empty - wmctrl will handle XWayland windows
        // Native Wayland windows (like Kitty) won't appear without an extension
        Ok(Vec::new())
    }

    fn get_windows_sway() -> Result<Vec<OpenWindow>, Box<dyn std::error::Error>> {
        let output = Command::new("swaymsg")
            .arg("-t")
            .arg("get_tree")
            .output()?;
        
        if !output.status.success() {
            return Err("swaymsg failed".into());
        }
        
        // Parse JSON (simplified - would need serde_json for full parsing)
        // For now, return empty or use a simpler approach
        // This is a placeholder - full implementation would parse the JSON tree
        Ok(Vec::new())
    }

    fn get_windows_hyprland() -> Result<Vec<OpenWindow>, Box<dyn std::error::Error>> {
        let output = Command::new("hyprctl")
            .arg("clients")
            .arg("-j")
            .output()?;
        
        if !output.status.success() {
            return Err("hyprctl failed".into());
        }
        
        // Parse JSON (simplified - would need serde_json for full parsing)
        // This is a placeholder
        Ok(Vec::new())
    }

    pub fn search<'a>(&self, query: &str, windows: &'a [OpenWindow]) -> Vec<(&'a OpenWindow, i64)> {
        if query.is_empty() {
            return windows.iter().map(|w| (w, 0)).collect();
        }

        let query_lower = query.to_lowercase();
        let mut results: Vec<(&OpenWindow, i64)> = Vec::new();

        for window in windows {
            // Match against title
            let title_score = self.matcher.fuzzy_match(&window.title.to_lowercase(), &query_lower);
            
            // Match against app name
            let app_score = self.matcher.fuzzy_match(&window.app_name.to_lowercase(), &query_lower);

            let score = title_score.unwrap_or(0).max(app_score.unwrap_or(0));

            if score > 0 {
                results.push((window, score));
            }
        }

        // Sort by score
        results.sort_unstable_by(|a, b| b.1.cmp(&a.1));
        results
    }

    pub fn switch_to_window(window: &OpenWindow) -> Result<(), Box<dyn std::error::Error>> {
        let is_gnome = std::env::var("XDG_CURRENT_DESKTOP")
            .map(|s| s.contains("GNOME"))
            .unwrap_or(false);
        
        let is_wayland = std::env::var("WAYLAND_DISPLAY").is_ok() || 
                         std::env::var("XDG_SESSION_TYPE").map(|s| s == "wayland").unwrap_or(false);
        
        // For GNOME Wayland, try D-Bus method first (for native Wayland windows)
        if is_gnome && is_wayland && !window.window_id.starts_with("0x") {
            // Try to activate window using GNOME Shell D-Bus
            // The window_id from GNOME is a stable sequence number
            let js_cmd = format!(
                "global.get_window_actors().find(a => a.meta_window.get_stable_sequence().toString() === '{}')?.meta_window.activate(global.get_current_time()); true",
                window.window_id
            );
            
            let output = Command::new("gdbus")
                .arg("call")
                .arg("--session")
                .arg("--dest")
                .arg("org.gnome.Shell")
                .arg("--object-path")
                .arg("/org/gnome/Shell")
                .arg("--method")
                .arg("org.gnome.Shell.Eval")
                .arg(&js_cmd)
                .output();
            
            if let Ok(output) = output {
                if output.status.success() {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    if stdout.contains("true") {
                        return Ok(());
                    }
                }
            }
        }
        
        // For GNOME, use wmctrl (works for XWayland windows)
        // wmctrl window IDs are hex format (0x...)
        let window_id = if window.window_id.starts_with("0x") {
            window.window_id.clone()
        } else {
            // Convert decimal to hex if needed
            if let Ok(id_num) = window.window_id.parse::<u64>() {
                format!("0x{:x}", id_num)
            } else {
                window.window_id.clone()
            }
        };
        
        // Try wmctrl first (most reliable for GNOME)
        if Self::has_command("wmctrl") {
            // Try with -i flag first
            let output = Command::new("wmctrl")
                .arg("-i")
                .arg("-a")
                .arg(&window_id)
                .output();
            
            if let Ok(output) = output {
                if output.status.success() {
                    return Ok(());
                }
            }
            
            // If that failed, try without -i (some wmctrl versions)
            let output2 = Command::new("wmctrl")
                .arg("-a")
                .arg(&window_id)
                .output();
            
            if let Ok(output2) = output2 {
                if output2.status.success() {
                    return Ok(());
                }
            }
        }
        
        // Fallback: try xdotool (convert hex to decimal)
        if Self::has_command("xdotool") {
            let xdotool_id = if window_id.starts_with("0x") {
                if let Ok(id_num) = u64::from_str_radix(&window_id[2..], 16) {
                    id_num.to_string()
                } else {
                    return Err("Invalid window ID format".into());
                }
            } else {
                window.window_id.clone()
            };
            
            if Command::new("xdotool")
                .arg("windowactivate")
                .arg(&xdotool_id)
                .output()?
                .status
                .success() {
                return Ok(());
            }
        }
        
        Err(format!("Could not switch to window: {}", window.title).into())
    }

    fn has_command(cmd: &str) -> bool {
        // Use command -v which is faster and more portable than which
        Command::new("sh")
            .arg("-c")
            .arg(&format!("command -v {}", cmd))
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}

