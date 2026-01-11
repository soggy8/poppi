# Poppi Launcher Architecture

## Project Structure

```
poppi_launcher/
├── src/
│   ├── main.rs           # Entry point, initializes GTK application
│   ├── ui.rs             # Main UI and window management
│   ├── app_launcher.rs   # Application discovery and launching
│   ├── calculator.rs     # Calculator functionality
│   ├── emoji_picker.rs   # Emoji search and insertion
│   ├── terminal.rs       # Terminal command execution
│   ├── search.rs         # Web search (YouTube, ChatGPT, Google)
│   ├── config.rs         # Configuration management (TOML)
│   └── utils.rs          # Utility functions (fuzzy matching helpers)
├── Cargo.toml            # Rust dependencies
├── README.md             # User documentation
└── poppi-launcher.desktop # Desktop entry file
```

## Core Components

### 1. Application Launcher (`app_launcher.rs`)
- Scans standard desktop entry directories:
  - `/usr/share/applications`
  - `/usr/local/share/applications`
  - `~/.local/share/applications`
- Parses `.desktop` files using INI format
- Fuzzy search through application names and descriptions
- Launches applications using `gio launch` (fallback to direct execution)

### 2. Calculator (`calculator.rs`)
- Uses `meval` crate for expression evaluation
- Supports: `+`, `-`, `*`, `/`, parentheses
- Automatically detects mathematical expressions
- Formats results (integers vs decimals)

### 3. Emoji Picker (`emoji_picker.rs`)
- Predefined emoji database with names and keywords
- Fuzzy search through emoji names and keywords
- Inserts emojis using `xdotool` (X11) - types into active window
- Prefix: `emoji` or `:`

### 4. Terminal Commands (`terminal.rs`)
- Detects common terminal commands
- Auto-detects terminal emulator (gnome-terminal, tilix, alacritty, etc.)
- Executes commands in a new terminal window

### 5. Web Search (`search.rs`)
- YouTube: `yt <query>` or `youtube <query>`
- ChatGPT: `gpt <query>` or `chatgpt <query>`
- Google: `google <query>`
- Opens URLs in default browser using `xdg-open`

### 6. Configuration (`config.rs`)
- TOML-based configuration
- Stored in `~/.config/poppi_launcher/config.toml`
- Customizable:
  - Theme (colors, fonts, size)
  - Shortcuts
  - Search engines
  - Calculator settings

### 7. UI (`ui.rs`)
- GTK4-based interface
- Minimal, modern design
- CSS styling support
- Real-time search with fuzzy matching
- Keyboard navigation (Enter, Escape, Arrow keys)

## Data Flow

1. **User Input** → Entry widget
2. **Query Processing** → `LauncherState::update_query()`
   - Determines mode (Apps, Calculator, Emoji, Terminal, Search)
   - Updates results list
3. **Results Display** → ListBox with filtered results
4. **Execution** → `LauncherState::execute_selected()`
   - Launches app / Calculates / Inserts emoji / Runs command / Opens URL
5. **Window Close** → After execution or Escape key

## Search Modes

The launcher automatically detects the intent based on query prefixes and content:

1. **Apps** (default): Normal text search
2. **Calculator**: Contains numbers and operators
3. **Emoji**: Starts with `emoji ` or `:`
4. **Terminal**: Starts with known command (ls, git, etc.)
5. **Search**: Starts with `yt`, `youtube`, `gpt`, `chatgpt`, or `google`

## Dependencies

### Rust Crates
- `gtk4` (0.9): GUI framework
- `glib`, `gio`: GTK4 support libraries
- `fuzzy-matcher`: Fast fuzzy search
- `ini`: Desktop entry parsing
- `meval`: Mathematical expression evaluation
- `toml`: Configuration file format
- `serde`: Serialization/deserialization
- `dirs`: System directories
- `urlencoding`: URL encoding for web searches

### System Dependencies
- GTK4 development libraries
- Pango, Cairo, GLib development libraries
- Optional: `xdotool` (emoji insertion), `xclip` (clipboard)

## Future Enhancements

- Wayland support for emoji insertion
- Plugin system
- Search history
- File search
- Custom command aliases
- Keyboard shortcut configuration UI
- Flatpak packaging

