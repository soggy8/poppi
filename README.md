# Poppi Launcher

A fast, modern, and customizable application launcher for the GNOME desktop environment, built with Rust and GTK4. Inspired by Albert launcher, Poppi provides a minimal UI with powerful search capabilities.

## Features

- **Application Search & Launch**: Fast fuzzy search through installed desktop applications
- **Calculator**: Built-in calculator for quick computations (e.g., `2+2`, `10*5`)
- **Emoji Picker**: Search and insert emojis directly into the active text field (prefix: `emoji` or `:`)
- **Terminal Commands**: Execute terminal commands directly (opens terminal and runs command)
- **Web Search**: Quick searches on YouTube (`yt`), ChatGPT (`gpt`), and Google (`google`)
- **Customizable**: Theme, colors, fonts, and shortcuts are all configurable

## Prerequisites

### System Dependencies

On Fedora/RHEL:
```bash
sudo dnf install gtk4-devel pango-devel cairo-devel glib2-devel gdk-pixbuf2-devel
```

On Ubuntu/Debian:
```bash
sudo apt install libgtk-4-dev libpango1.0-dev libcairo2-dev libglib2.0-dev libgdk-pixbuf2.0-dev
```

On Arch Linux:
```bash
sudo pacman -S gtk4 pango cairo glib2 gdk-pixbuf2
```

### Rust

Make sure you have Rust installed. If not:
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

### Optional Dependencies

For emoji insertion (X11):
```bash
# Fedora/RHEL
sudo dnf install xdotool

# Ubuntu/Debian
sudo apt install xdotool

# Arch Linux
sudo pacman -S xdotool
```

For clipboard functionality:
```bash
# Fedora/RHEL
sudo dnf install xclip

# Ubuntu/Debian
sudo apt install xclip

# Arch Linux
sudo pacman -S xclip
```

## Building

```bash
git clone <repository-url>
cd poppi_launcher
cargo build --release
```

The binary will be in `target/release/poppi_launcher`.

## Usage

### Running

```bash
cargo run --release
# or
./target/release/poppi_launcher
```

### Keyboard Shortcuts

- **Enter**: Execute the selected item
- **Escape**: Close the launcher
- **Arrow Keys**: Navigate through results

### Search Modes

1. **Applications**: Just type the app name (default mode)
2. **Calculator**: Type a mathematical expression (e.g., `2+2`, `10*5-3`)
3. **Emoji**: Prefix with `emoji` or `:` (e.g., `emoji smile`, `:heart`)
4. **Terminal**: Type a terminal command (e.g., `ls`, `git status`)
5. **Web Search**: 
   - YouTube: `yt <query>` or `youtube <query>`
   - ChatGPT: `gpt <query>` or `chatgpt <query>`
   - Google: `google <query>`

## Configuration

Configuration is stored in `~/.config/poppi_launcher/config.toml`. The configuration file is automatically created on first run with default values.

Example configuration:

```toml
[theme]
background_color = "#2e2e2e"
text_color = "#ffffff"
accent_color = "#5294e2"
border_radius = 12.0
font_size = 16
width = 800
height = 600

[shortcuts]
show_launcher = "Super+Space"

[search]
default_engine = "google"
youtube_enabled = true
chatgpt_enabled = true

[calculator]
enabled = true
```

### Customization Options

- **Theme Colors**: Customize background, text, and accent colors
- **Window Size**: Adjust width and height
- **Font Size**: Change the font size
- **Border Radius**: Round the window corners
- **Search Engines**: Enable/disable YouTube and ChatGPT search

## Features in Detail

### Application Launcher

Searches through all installed desktop applications using fuzzy matching. Results are sorted by relevance.

### Calculator

Supports basic arithmetic operations:
- Addition: `+`
- Subtraction: `-`
- Multiplication: `*` or `×`
- Division: `/` or `÷`
- Parentheses for grouping

### Emoji Picker

Search emojis by name or keywords. Selected emojis are automatically inserted into the currently focused text field using `xdotool` (X11) or clipboard (fallback).

### Terminal Commands

Recognizes common terminal commands and executes them in your default terminal emulator. The launcher detects:
- `gnome-terminal`
- `tilix`
- `alacritty`
- `kitty`
- `konsole`
- `xterm`
- And more...

### Web Search

Quick access to:
- **YouTube**: Search videos directly
- **ChatGPT**: Open ChatGPT (query can be pasted)
- **Google**: Web search

## Limitations

- Emoji insertion requires `xdotool` on X11. Wayland support is planned.
- Terminal command detection uses heuristics and may not recognize all commands.
- Desktop entry parsing is simplified and may not handle all edge cases.

## Roadmap

- [ ] Wayland support for emoji insertion
- [ ] More sophisticated desktop entry parsing
- [ ] Plugin system for extending functionality
- [ ] History of recent searches and launches
- [ ] File search
- [ ] Custom command aliases
- [ ] Keyboard shortcut configuration UI
- [ ] Flatpak packaging

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

## License

[Add your license here]

## Acknowledgments

- Inspired by [Albert Launcher](https://github.com/albertlauncher/albert)
- Built with [GTK4](https://www.gtk.org/) and [Rust](https://www.rust-lang.org/)

