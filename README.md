# 🚀 Poppi Launcher

<div align="center">

![Rust](https://img.shields.io/badge/Rust-1.73%2B-orange?logo=rust)
![GTK4](https://img.shields.io/badge/GTK4-4.12%2B-blue?logo=gnome)
![License](https://img.shields.io/badge/License-MIT-green)
![Platform](https://img.shields.io/badge/Platform-Linux-lightgrey?logo=linux)

**A fast, modern, and customizable application launcher for GNOME desktop environment**

Inspired by Albert launcher, built with Rust and GTK4 for optimal performance

[Features](#-features) • [Installation](#-installation) • [Usage](#-usage) • [Configuration](#-configuration) • [Contributing](#-contributing)

</div>

---

## ✨ Features

- **🎯 Applications**: Search installed apps, including Flatpak and Snap. Matches names, keywords and descriptions; the apps you launch most rank higher and are listed as soon as the launcher opens
- **📁 Files**: Find files by name (`f report`) or browse by typing a path (`~/Doc`, Tab to complete)
- **🪟 Window Switcher**: Switch between open windows, shown with their app icons (prefix: `sw` or `switch`)
- **🔢 Calculator**: Type an expression such as `2+2`, `3x4` or `sqrt(16)`; Enter copies the result
- **😀 Emoji Picker**: Search emojis and insert them into the window you came from (prefix: `emoji` or `:`)
- **💻 Terminal Commands**: Run a command in your terminal (prefix: `>`, `$`, `!`, `term` or `cmd`)
- **🌐 Web Search**: YouTube (`yt`), ChatGPT (`gpt`), Google (`google`) and DuckDuckGo (`ddg`)
- **🤖 Ask Claude**: End your text with `?` and press Enter to get an answer from Claude, right in the launcher. Uses your Claude subscription through Claude Code; no API key
- **🔗 Aliases**: Your own keywords for commands and URLs, which doubles as custom search engines
- **🕘 History**: Step back through past queries with Alt+Up, or list them with `history`
- **⚡ Instant**: Stays resident after the first start, so the shortcut toggles the window immediately
- **🎨 Customizable**: Colors, font size, corner radius, width and behavior via TOML or the settings window

## 🛠️ Installation

### Prerequisites

GTK 4.12 or newer and a Rust toolchain (1.73+).

```bash
# Fedora/RHEL
sudo dnf install gtk4-devel

# Ubuntu/Debian
sudo apt install libgtk-4-dev

# Arch Linux
sudo pacman -S gtk4
```

#### Optional tools

| Feature | Wayland | X11 |
|---|---|---|
| Copy calculator results, emoji fallback | `wl-clipboard` | `xclip` or `xsel` |
| Type emojis into the focused window | `wtype` (wlroots compositors) or `ydotool` | `xdotool` |
| Window switcher | [window-calls](https://github.com/ickyicky/window-calls) GNOME extension | `wmctrl` (plus `xprop`) or `xdotool` |

On GNOME Wayland no tool can type into another window, so emojis are copied to the clipboard instead; paste with Ctrl+V.

### Build and install

```bash
git clone https://github.com/soggy8/poppi.git
cd poppi
make install            # installs to ~/.local; use PREFIX=/usr/local for system-wide
```

This installs the binary, the desktop entry and the icon. `make uninstall` removes them. Make sure `~/.local/bin` is on your `PATH`.

### Keyboard shortcut

Poppi does not grab a global key itself; bind one in your desktop. On GNOME:

```bash
scripts/setup-gnome-shortcut.sh '<Control>space'
```

or add a custom shortcut by hand in Settings → Keyboard → View and Customize Shortcuts → Custom Shortcuts with the command `poppi_launcher`.

## 🚀 Usage

Run `poppi_launcher`. Running it again while the window is open closes the window, so one shortcut toggles it.

By default the process keeps running in the background after the window closes, which makes later invocations instant. Set `stay_resident = false` to make it exit instead. After installing a new build, stop the old instance with `pkill poppi_launcher`.

### Keys

| Key | Action |
|---|---|
| Enter | Run the highlighted item |
| Ctrl+Enter | For a file: open its folder instead |
| Up / Down, Tab / Shift+Tab | Move through results (the list scrolls); scroll a Claude answer |
| Page Up / Page Down | Move a page at a time |
| Left / Right | Also move in the emoji grid |
| Tab | While typing a path: complete it with the highlighted entry |
| Alt+Up / Alt+Down | Step through past queries |
| Escape | Close the launcher |

The highlighted row shows on its right what Enter will do (Launch, Open, Copy, …).

### Search Modes

1. **Applications** (default): just type. `firefox`, `code`, `vsc` (initials), `frfx` (typo-tolerant)
   - With an empty bar, your most launched apps are listed.
   - Apps always win over commands with the same name.
   - If nothing matches, web searches are offered.

2. **Files**: `f report`, `file annual report`
   - Searches file and folder names under the configured roots (your home directory by default). The index is built in the background on first use and refreshed every few minutes.
   - Type a path to browse instead: `~/`, `~/Doc`, `/etc/`. Tab completes, Enter opens.

3. **Window Switcher**: `sw` or `switch`, optionally followed by a filter. `sw kit`

4. **Calculator**: `2+2`, `10*5-3`, `(5+3)*2`, `2^10`, `3x4`, `sqrt(2)`, `max(3, 7)`, `2*pi`
   - Operators: `+ - * / % ^`, with `x`, `×` and `÷` as aliases
   - Functions: `sqrt abs sin cos tan asin acos atan ln log log2 exp floor ceil round min max`

5. **Emoji**: `emoji smile`, `:heart`

6. **Terminal Commands**: `> ls -la`, `$ git status`, `! htop`, `term cargo build`, `cmd df -h`
   - Opens your terminal (`$TERMINAL`, then the system default) and keeps it open after the command finishes.
   - Without a prefix, a command found on `$PATH` is listed after any matching apps.

7. **Web Search**: `yt rust tutorial`, `gpt explain lifetimes`, `google rust gtk4`, `ddg privacy`

8. **Aliases**: whatever you define under `[aliases]` in the config (see below). `gh gtk4 rust`

9. **History**: `history` lists past queries (Enter puts one back in the bar), `history yt` filters, `history clear` forgets them. App launches and window switches are not recorded.

10. **Settings**: `poppi settings`, or just `settings`

### Asking Claude

Anything that ends with a question mark is a question for Claude:

```
why is the sky blue?
what's the tar command to extract a .tar.gz?
```

Press Enter to send it. The answer streams into the launcher; Up/Down scroll a long answer and Enter copies it. Nothing is sent while you type, and editing the text or closing the launcher cancels a request in flight.

**Setup.** This uses the [Claude Code](https://claude.com/claude-code) CLI and whatever account it is logged in with, so it runs on your Pro or Max subscription:

```bash
claude        # then /login once, choosing your Claude subscription
```

There is no API key setting. The launcher runs `claude -p` and removes `ANTHROPIC_API_KEY` from that process's environment, so an exported key is never billed instead. Questions count against your subscription's normal usage limits.

The CLI is started with all tools, MCP servers and session saving turned off: Claude can only reply with text and cannot read files or run commands. The default model is Haiku, the fastest; change it under `[claude]` or in the settings window.

## ⚙️ Configuration

Configuration lives in `~/.config/poppi_launcher/config.toml` and is created with defaults on first run. Every key is optional. If the file cannot be parsed, the launcher says so in the search bar, prints the reason to stderr and uses defaults without overwriting your file.

```toml
[theme]
background_color = "#0a0a0a"
text_color = "#e0e0e0"
accent_color = "#4a9eff"
border_radius = 0.0      # corner radius in px
font_size = 16           # pt
width = 600              # px; height follows the results

[behavior]
close_on_focus_loss = true
stay_resident = true     # keep running in the background for instant startup
max_results = 5          # rows visible at once; longer lists scroll
history_size = 50        # remembered queries, 0 = off

[search]
default_engine = "google"   # google, duckduckgo, youtube or chatgpt
youtube_enabled = true
chatgpt_enabled = true

[calculator]
enabled = true

[claude]
enabled = true
model = "haiku"          # or "sonnet", "opus", or a full model name
command = "claude"       # the Claude Code executable
system_prompt = "You answer questions inside a small desktop launcher. Reply in plain text without markdown, as briefly as the question allows: usually one to three sentences."

[files]
enabled = true
roots = ["~"]            # directories to index
max_depth = 6            # levels below each root
include_hidden = false
exclude = ["node_modules", "target", "__pycache__", "venv", "snap"]

[aliases]
# keyword = "URL or command"; {} is replaced by what you type after the keyword
gh = "https://github.com/search?q={}"
wiki = "https://en.wikipedia.org/wiki/Special:Search?search={}"
up = "kitty -e sudo dnf upgrade"
```

Colors accept anything GTK CSS understands (`#rrggbb`, `rgba(...)`, names). The settings window edits most of these values and applies them immediately; aliases and file roots are edited in the file (the settings window has a button that opens it).

**Aliases.** The first word you type is looked up in `[aliases]`. A value starting with `http://` or `https://` is opened in the browser with the rest of your input URL-encoded in place of `{}`, which makes it a custom search engine. Anything else is run as a shell command, with your input inserted (each word quoted) at `{}` or appended. An alias may reuse a built-in keyword such as `yt` to replace it.

**History and usage data** live in `~/.local/state/poppi_launcher/`. Set `history_size = 0` under `[behavior]` to turn history off.

## 📋 Requirements

- **Rust**: 1.73 or later
- **GTK4**: 4.12 or later
- **Linux**: built for GNOME; other GTK4-capable desktops work with the X11 or wlroots tools listed above

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
