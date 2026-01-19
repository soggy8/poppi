# 🚀 Poppi Launcher

<div align="center">

![Rust](https://img.shields.io/badge/Rust-1.70%2B-orange?logo=rust)
![GTK4](https://img.shields.io/badge/GTK4-4.12%2B-blue?logo=gnome)
![License](https://img.shields.io/badge/License-MIT-green)
![Platform](https://img.shields.io/badge/Platform-Linux-lightgrey?logo=linux)

**A fast, modern, and customizable application launcher for GNOME desktop environment**

Inspired by Albert launcher, built with Rust and GTK4 for optimal performance

[Features](#-features) • [Installation](#-installation) • [Usage](#-usage) • [Configuration](#-configuration) • [Contributing](#-contributing)

</div>

---

## ✨ Features

- **🎯 Application Search & Launch**: Fast fuzzy search through installed desktop applications
- **🪟 Window Switcher**: Switch between open windows like GNOME's Super+Tab (prefix: `sw` or `switch`)
- **🔢 Calculator**: Built-in calculator for quick computations (e.g., `2+2`, `10*5`)
- **😀 Emoji Picker**: Search and insert emojis directly into the active text field (prefix: `emoji` or `:`)
- **💻 Terminal Commands**: Execute terminal commands directly (opens terminal and runs command)
- **🌐 Web Search**: Quick searches on YouTube (`yt`), ChatGPT (`gpt`), and Google (`google`)
- **⚡ Fast & Lightweight**: Built with Rust for optimal performance
- **🎨 Customizable**: Theme, colors, fonts, and shortcuts are all configurable via TOML
- **📐 Compact UI**: Minimal search bar that dynamically expands to show results
- **⌨️ Keyboard Navigation**: Full arrow key support for navigation

## 📸 Screenshots

*Coming soon - screenshots of Poppi Launcher in action*

## 🛠️ Installation

### Prerequisites

#### System Dependencies

**On Fedora/RHEL:**
```bash
sudo dnf install gtk4-devel pango-devel cairo-devel glib2-devel gdk-pixbuf2-devel
```

**On Ubuntu/Debian:**
```bash
sudo apt install libgtk-4-dev libpango1.0-dev libcairo2-dev libglib2.0-dev libgdk-pixbuf2.0-dev
```

**On Arch Linux:**
```bash
sudo pacman -S gtk4 pango cairo glib2 gdk-pixbuf2
```

#### Rust

Make sure you have Rust installed. If not:
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

#### Optional Dependencies

For window switching:
```bash
# Fedora/RHEL
sudo dnf install wmctrl

# Ubuntu/Debian
sudo apt install wmctrl

# Arch Linux
sudo pacman -S wmctrl
```

For native Wayland window support (recommended):
- Install the [window-calls](https://github.com/ickyicky/window-calls) GNOME Shell extension
- This enables switching to native Wayland applications (Kitty, Zen, etc.)

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

### Building from Source

```bash
git clone https://github.com/soggy8/poppi.git
cd poppi
cargo build --release
```

The binary will be in `target/release/poppi_launcher`.

### Installation

After building, you can install it system-wide:

```bash
sudo cp target/release/poppi_launcher /usr/local/bin/
```

Or add it to your local bin:

```bash
mkdir -p ~/.local/bin
cp target/release/poppi_launcher ~/.local/bin/
export PATH="$HOME/.local/bin:$PATH"  # Add to your ~/.bashrc or ~/.zshrc
```

## 🚀 Usage

### Running

```bash
poppi_launcher
# or
./target/release/poppi_launcher
```

### Keyboard Shortcuts

- **Enter**: Execute the selected item
- **Escape**: Close the launcher
- **Arrow Up/Down**: Navigate through results
- **Click**: Launch an application by clicking on it

### Search Modes

1. **Applications** (default): Just type the app name
   - Example: `firefox`, `code`, `terminal`

2. **Window Switcher**: Prefix with `sw` or `switch`
   - Example: `sw`, `switch`, `sw kit` (to filter for Kitty)
   - Shows all open windows (XWayland and native Wayland if window-calls extension is installed)
   - Filters out system windows, popups, and utility windows
   - Works like GNOME's Super+Tab switcher

3. **Calculator**: Type a mathematical expression
   - Example: `2+2`, `10*5-3`, `(5+3)*2`

4. **Emoji**: Prefix with `emoji` or `:`
   - Example: `emoji smile`, `:heart`, `emoji fire`

5. **Terminal Commands**: Type any terminal command (automatically detected if command exists in PATH)
   - Example: `ls`, `git status`, `docker ps`, `cargo build`, `npm install`
   - Or use explicit prefixes: `> command`, `$ command`, `! command`, `term command`, `cmd command`
   - Supports ALL commands in your PATH, not just hardcoded ones

6. **Web Search**: 
   - **YouTube**: `yt <query>` or `youtube <query>`
     - Example: `yt rust tutorial`
   - **ChatGPT**: `gpt <query>` or `chatgpt <query>`
     - Example: `gpt hello`
   - **Google**: `google <query>`
     - Example: `google rust programming`

## ⚙️ Configuration

Configuration is stored in `~/.config/poppi_launcher/config.toml`. The configuration file is automatically created on first run with default values.

### Example Configuration

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
- **Window Size**: Adjust width and height (for expanded view)
- **Font Size**: Change the font size
- **Border Radius**: Round the window corners
- **Search Engines**: Enable/disable YouTube and ChatGPT search

## 📋 Requirements

- **Rust**: 1.70 or later
- **GTK4**: 4.12 or later
- **GNOME**: Desktop environment (or any GTK4-compatible desktop)
- **Linux**: Tested on Fedora, Ubuntu, and Arch Linux

## 🏗️ Architecture

See [ARCHITECTURE.md](ARCHITECTURE.md) for detailed information about the project structure and architecture.

## 🤝 Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

1. Fork the repository
2. Create your feature branch (`git checkout -b feature/amazing-feature`)
3. Commit your changes (`git commit -m 'Add some amazing feature'`)
4. Push to the branch (`git push origin feature/amazing-feature`)
5. Open a Pull Request

## 📝 License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## 🙏 Acknowledgments

- Inspired by [Albert Launcher](https://github.com/albertlauncher/albert)
- Built with [GTK4](https://www.gtk.org/) and [Rust](https://www.rust-lang.org/)
- Thanks to the open-source community for amazing tools and libraries

## 📧 Contact

- **GitHub**: [@soggy8](https://github.com/soggy8)
- **Repository**: https://github.com/soggy8/poppi

---

<div align="center">

**Made with ❤️ and Rust**

⭐ Star this repo if you find it useful!

</div>
