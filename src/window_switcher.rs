use std::process::Command;
use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;
use serde_json::Value;

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
        let mut all_windows = Vec::new();
        
        // Try window-calls extension first (for native Wayland windows)
        if let Ok(mut windows) = Self::get_windows_via_window_calls() {
            all_windows.append(&mut windows);
        }
        
        // Get XWayland windows with wmctrl (most reliable for GNOME)
        if Self::has_command("wmctrl") {
            match Self::get_windows_wmctrl_filtered() {
                Ok(mut windows) => {
                    // Merge with existing windows, avoiding duplicates by title+app
                    let existing_keys: std::collections::HashSet<(String, String)> = 
                        all_windows.iter()
                            .map(|w| (w.title.clone(), w.app_name.clone()))
                            .collect();
                    for w in windows {
                        let key = (w.title.clone(), w.app_name.clone());
                        if !existing_keys.contains(&key) {
                            all_windows.push(w);
                        }
                    }
                }
                Err(e) => eprintln!("wmctrl failed: {}", e), // Log but continue
            }
        }
        
        // Note: Native Wayland windows require window-calls extension
        // Process-based detection doesn't work for switching
        
        // If we have windows, return them
        if !all_windows.is_empty() {
            return Ok(all_windows);
        }
        
        // Fallback: try xdotool (but filter heavily)
        if Self::has_command("xdotool") {
            match Self::get_windows_xdotool_filtered() {
                Ok(windows) if !windows.is_empty() => return Ok(windows),
                Ok(_) => {}, // Empty result
                Err(e) => eprintln!("xdotool failed: {}", e), // Log but continue
            }
        }
        
        Err("No windows found. Install 'window-calls' extension for native Wayland window support.".into())
    }
    
    fn should_try_process_detection() -> bool {
        // Only try if we're on Wayland and have few windows
        std::env::var("WAYLAND_DISPLAY").is_ok() || 
        std::env::var("XDG_SESSION_TYPE").map(|s| s == "wayland").unwrap_or(false)
    }
    
    fn get_windows_by_process() -> Result<Vec<OpenWindow>, Box<dyn std::error::Error>> {
        // This method doesn't work well for switching since we don't have real window IDs
        // It's better to just return empty and let the user know they need window-calls
        // for native Wayland windows
        Ok(Vec::new())
    }
    
    fn get_windows_via_window_calls() -> Result<Vec<OpenWindow>, Box<dyn std::error::Error>> {
        // Try to use window-calls extension if available
        let output = Command::new("gdbus")
            .arg("call")
            .arg("--session")
            .arg("--dest")
            .arg("org.gnome.Shell")
            .arg("--object-path")
            .arg("/org/gnome/Shell/Extensions/Windows")
            .arg("--method")
            .arg("org.gnome.Shell.Extensions.Windows.List")
            .output();
        
        let output = match output {
            Ok(o) => o,
            Err(_) => return Ok(Vec::new()), // Extension not available, not an error
        };
        
        if !output.status.success() {
            return Ok(Vec::new()); // Extension not available
        }
        
        let stdout = String::from_utf8_lossy(&output.stdout);
        
        // Parse the output - gdbus returns something like: (av, [array of windows])
        // The array contains dictionaries with window info
        // Format: ([{...}, {...}],)
        // We need to extract the JSON array
        
        // Try to find JSON array in the output
        if let Some(start) = stdout.find('[') {
            if let Some(end) = stdout.rfind(']') {
                let json_str = &stdout[start..=end];
                
                // Parse JSON array
                let windows_json: Vec<Value> = match serde_json::from_str(json_str) {
                    Ok(w) => w,
                    Err(_) => return Ok(Vec::new()), // Invalid JSON
                };
                
                let mut windows = Vec::new();
                for win in windows_json {
                    // Get window properties
                    let id = win.get("id").and_then(|v| v.as_u64());
                    let title = win.get("title").and_then(|v| v.as_str());
                    let wm_class = win.get("wm_class").and_then(|v| v.as_str());
                    let window_type = win.get("window_type").and_then(|v| v.as_u64()).unwrap_or(999);
                    let in_current_workspace = win.get("in_current_workspace").and_then(|v| v.as_bool()).unwrap_or(false);
                    
                    // Only include normal windows (window_type == 0)
                    if window_type != 0 {
                        continue;
                    }
                    
                    // Optionally filter by workspace - only show current workspace windows
                    // You can change this to show all workspaces if desired
                    // if !in_current_workspace {
                    //     continue;
                    // }
                    
                    if let (Some(id), Some(title), wm_class) = (id, title, wm_class) {
                        let title = title.to_string();
                        let app_name = wm_class
                            .map(|c| {
                                let class_name = c.split('.').next().unwrap_or(c);
                                if let Some(first_char) = class_name.chars().next() {
                                    if class_name.len() > 1 {
                                        format!("{}{}", first_char.to_uppercase(), &class_name[1..])
                                    } else {
                                        first_char.to_uppercase().to_string()
                                    }
                                } else {
                                    class_name.to_string()
                                }
                            })
                            .unwrap_or_else(|| {
                                title.split_whitespace().next().unwrap_or("Window").to_string()
                            });
                        
                        // Filter out system windows
                        if Self::should_include_window(&title, &app_name) {
                            windows.push(OpenWindow {
                                window_id: id.to_string(), // Use decimal ID for Wayland windows
                                title,
                                app_name,
                            });
                        }
                    }
                }
                
                return Ok(windows);
            }
        }
        
        Ok(Vec::new())
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
            "mutter guard",
            "notification",
            "popup",
            "tooltip",
            "dropdown",
            "menu",
            "poppi_launcher",
            "guard window",
            "splash",
            "dock",
            "panel",
            "tray",
            "overlay",
            "compositor",
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
                
                // Check window type using xprop if available (to filter out DOCK, DESKTOP, etc.)
                let should_include = if Self::has_command("xprop") {
                    Self::is_normal_window(&window_id)
                } else {
                    true // If xprop not available, use basic filtering
                };
                
                // Filter windows
                if should_include && Self::should_include_window(&title, &app_name) {
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
        // Get only visible windows to avoid system windows
        let output = Command::new("xdotool")
            .arg("search")
            .arg("--onlyvisible")
            .arg("--name")
            .arg(".")
            .output()?;
        
        if !output.status.success() {
            return Err("xdotool search failed".into());
        }
        
        let window_ids: Vec<String> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .take(50) // Limit to 50 windows for performance
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
                    if title.is_empty() || title == "N/A" || title.len() < 3 {
                        continue;
                    }
                    
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
                                    let class_name = name.split('.').next().unwrap_or(&name);
                                    // Convert to title case
                                    if let Some(first_char) = class_name.chars().next() {
                                        Some(format!("{}{}", first_char.to_uppercase(), &class_name[1..]))
                                    } else {
                                        Some(class_name.to_string())
                                    }
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
                    
                    // Filter windows aggressively
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
            // Return all windows, sorted by title
            let mut all: Vec<(&OpenWindow, i64)> = windows.iter().map(|w| (w, 0)).collect();
            all.sort_by(|a, b| a.0.title.cmp(&b.0.title));
            return all;
        }

        let query_lower = query.to_lowercase();
        let mut results: Vec<(&OpenWindow, i64)> = Vec::new();

        for window in windows {
            // Match against title (higher weight)
            let title_score = self.matcher.fuzzy_match(&window.title.to_lowercase(), &query_lower);
            
            // Match against app name
            let app_score = self.matcher.fuzzy_match(&window.app_name.to_lowercase(), &query_lower);

            // Weight title matches higher
            let score = title_score
                .map(|s| s * 2) // Double weight for title matches
                .unwrap_or(0)
                .max(app_score.unwrap_or(0));

            if score > 0 {
                results.push((window, score));
            }
        }

        // Sort by score (highest first)
        results.sort_unstable_by(|a, b| b.1.cmp(&a.1));
        results
    }

    pub fn switch_to_window(window: &OpenWindow) -> Result<(), Box<dyn std::error::Error>> {
        let is_gnome = std::env::var("XDG_CURRENT_DESKTOP")
            .map(|s| s.contains("GNOME"))
            .unwrap_or(false);
        
        let is_wayland = std::env::var("WAYLAND_DISPLAY").is_ok() || 
                         std::env::var("XDG_SESSION_TYPE").map(|s| s == "wayland").unwrap_or(false);
        
        // For native Wayland windows (from window-calls extension), use D-Bus Activate
        if !window.window_id.starts_with("0x") && !window.window_id.starts_with("wayland:") {
            // Try window-calls extension Activate method
            if let Ok(id_num) = window.window_id.parse::<u64>() {
                let output = Command::new("gdbus")
                    .arg("call")
                    .arg("--session")
                    .arg("--dest")
                    .arg("org.gnome.Shell")
                    .arg("--object-path")
                    .arg("/org/gnome/Shell/Extensions/Windows")
                    .arg("--method")
                    .arg("org.gnome.Shell.Extensions.Windows.Activate")
                    .arg(&id_num.to_string())
                    .output();
                
                if let Ok(output) = output {
                    if output.status.success() {
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
            // Try with -i flag first (activate by window ID)
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
            
            // If that failed, try without -i flag (some wmctrl versions)
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

    fn is_normal_window(window_id: &str) -> bool {
        // Check if window is a normal window type (not DOCK, DESKTOP, SPLASH, etc.)
        // Using xprop to check _NET_WM_WINDOW_TYPE
        let output = Command::new("xprop")
            .arg("-id")
            .arg(window_id)
            .arg("_NET_WM_WINDOW_TYPE")
            .output();
        
        if let Ok(output) = output {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                // Normal windows have _NET_WM_WINDOW_TYPE = _NET_WM_WINDOW_TYPE_NORMAL
                // We want to exclude: DOCK, DESKTOP, SPLASH, UTILITY, TOOLBAR, MENU, etc.
                if stdout.contains("_NET_WM_WINDOW_TYPE_NORMAL") {
                    return true;
                }
                // Also check for DIALOG which is usually fine
                if stdout.contains("_NET_WM_WINDOW_TYPE_DIALOG") {
                    return true;
                }
                // Exclude everything else
                return false;
            }
        }
        // If xprop fails, assume it's normal (fallback to basic filtering)
        true
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

