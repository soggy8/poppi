# 🚀 Poppi Launcher

<div align="center">

![Rust](https://img.shields.io/badge/Rust-stable-orange?logo=rust)
![GTK4](https://img.shields.io/badge/GTK4-4.12%2B-blue?logo=gnome)
![License](https://img.shields.io/badge/License-MIT-green)
![Platform](https://img.shields.io/badge/Platform-Linux-lightgrey?logo=linux)

**A fast, modern, and customizable application launcher for GNOME desktop environment**

Inspired by Albert launcher, built with Rust and GTK4 for optimal performance

[Features](#-features) • [Installation](#-installation) • [Usage](#-usage) • [Configuration](#-configuration) • [Contributing](#-contributing)

</div>

---

## ✨ Features

- **🎯 Applications**: Fuzzy search over installed apps (Flatpak and Snap included), with matched letters highlighted. Apps you use often and recently rank first and are listed as soon as the launcher opens. App actions such as "New Private Window" are searchable too
- **📁 Files**: Find files by name (`f report`), recently changed ones first, or browse by typing a path (`~/Doc`, Tab to complete)
- **🪟 Window Switcher**: Switch between open windows, most recently used first, shown with their app icons (`sw`)
- **🔢 Calculator and conversion**: `2+2`, `sqrt(16)`, `5 km in miles`, `100 f to c`, `20 usd in eur`
- **🤖 Ask Claude**: End your text with `?` and get an answer in the launcher, then keep typing follow-ups. Runs on your Claude subscription through Claude Code; no API key
- **😀 Emoji Picker**: Search emojis, recents first, with your skin tone (`:` or `emoji`)
- **📋 Clipboard history**: `clip` lists recently copied texts
- **⏻ System actions**: `lock`, `suspend`, `log out`, `restart`, `power off`
- **💻 Terminal Commands**: `> ls -la` runs a command in your terminal
- **🌐 Web Search**: YouTube (`yt`), ChatGPT (`gpt`), Google (`google`), DuckDuckGo (`ddg`)
- **🔗 Aliases**: Your own keywords for commands and URLs, which doubles as custom search engines
- **🕘 History**: Step back through past queries with Alt+Up, or list them with `history`
- **🎨 Themes**: Dark, light or follow the system, with your own colors, radius and size; changes apply live
- **⚡ Instant**: Stays resident after the first start, so the shortcut toggles the window immediately

## 🛠️ Installation

### Prerequisites

GTK 4.12 or newer and a recent stable Rust toolchain.

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
| Currency conversion | `curl` | `curl` |
| Ask Claude | [Claude Code](https://claude.com/claude-code), logged in | same |
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

Run `poppi_launcher`. Running it again while the window is open closes it, so one shortcut toggles it.

By default the process keeps running in the background after the window closes, which makes later invocations instant (set `stay_resident = false` to make it exit instead).

```
poppi_launcher            show or toggle the launcher
poppi_launcher --quit     stop the background instance (do this after installing a new build)
poppi_launcher --version
```

The chip on the right of the search bar names the mode your text is in, and the footer shows what the keys do for the highlighted result.

### Keys

| Key | Action |
|---|---|
| Enter | Run the highlighted item |
| Alt+1 … Alt+9 | Run the first to ninth result directly |
| Ctrl+Enter | For a file: open its folder instead |
| Up / Down, Tab / Shift+Tab | Move through results (the list scrolls) |
| Page Up / Page Down | Move a page at a time |
| Left / Right | Also move in the emoji grid |
| Tab | While typing a path: complete it with the highlighted entry |
| Alt+Up / Alt+Down | Step through past queries |
| Backspace on an empty bar | Leave a conversation with Claude |
| Escape | Close the launcher |

### Search Modes

1. **Applications** (default): just type. `firefox`, `code`, `vsc` (initials), `frfx` (typo-tolerant)
   - With an empty bar, the apps you use most are listed.
   - Apps always win over commands with the same name.
   - If nothing matches, web searches are offered.

2. **Files**: `f report`, `file annual report`
   - Searches file and folder names under the configured roots (your home directory by default). Recently modified files and ones you opened from here rank higher. The index is built in the background on first use and refreshed every few minutes.
   - Type a path to browse instead: `~/`, `~/Doc`, `/etc/`. Tab completes, Enter opens.

3. **Window Switcher**: `sw` or `switch`, optionally followed by a filter. `sw kit`

4. **Calculator**: `2+2`, `10*5-3`, `(5+3)*2`, `2^10`, `3x4`, `sqrt(2)`, `max(3, 7)`, `2*pi`
   - Operators: `+ - * / % ^`, with `x`, `×` and `÷` as aliases
   - Functions: `sqrt abs sin cos tan asin acos atan ln log log2 exp floor ceil round min max`
   - **Units**: `5 km in miles`, `10 cm to in`, `2 gb in mb`, `90 min in h`, `100 km/h in mph`, `100 f to c`. Length, mass, volume, data, time, speed, area and temperature.
   - **Currencies**: `20 usd in eur`, `1000 jpy to gbp`. Uses European Central Bank reference rates, downloaded from frankfurter.dev at most twice a day and cached. Turn this off with `currency = false` if you do not want that request made.

5. **Ask Claude**: anything ending in `?`, or starting with `ask ` or `ai `. See below.

6. **Emoji**: `emoji smile`, `:heart`. With nothing typed after the prefix, your recently used emojis come first.

7. **Clipboard**: `clip`, `cb` or `clipboard`, optionally with a filter. Enter copies the entry again.
   - Kept in memory only, never written to disk.
   - On GNOME Wayland an application can only read the clipboard while it has focus, so what gets remembered is whatever was on the clipboard each time the launcher was open. On X11 every copy is recorded.

8. **System**: `lock`, `suspend` (or `sleep`), `log out`, `restart` (or `reboot`), `power off` (or `shutdown`). On GNOME, the ones that end your session show GNOME's own confirmation dialog; elsewhere the launcher asks for a second Enter.

9. **Terminal Commands**: `> ls -la`, `$ git status`, `! htop`, `term cargo build`, `cmd df -h`
   - Opens your terminal (`$TERMINAL`, then the system default) and keeps it open after the command finishes.
   - Without a prefix, a command found on `$PATH` is listed after any matching apps.

10. **Web Search**: `yt rust tutorial`, `gpt explain lifetimes`, `google rust gtk4`, `ddg privacy`

11. **Aliases**: whatever you define in settings or under `[aliases]`. `gh gtk4 rust`

12. **History**: `history` lists past queries (Enter puts one back in the bar), `history yt` filters, `history clear` forgets them. App launches and window switches are not recorded.

13. **Settings**: `poppi settings`, or just `settings`

### Asking Claude

Anything that ends with a question mark is a question for Claude (so is anything after `ask ` or `ai `):

```
why is the sky blue?
what's the tar command to extract a .tar.gz?
ask write a haiku about GTK
```

Press Enter to send it. The answer streams into the launcher, with code, bold text and lists rendered.

After the first answer you are in a conversation: whatever you type next is a follow-up, with or without a question mark, and Claude remembers the earlier turns. Up/Down scroll, Enter on an empty bar copies the latest answer, Backspace on an empty bar goes back to normal search, and closing the launcher ends the conversation.

**Setup.** This uses the [Claude Code](https://claude.com/claude-code) CLI and whatever account it is logged in with, so it runs on your Pro or Max subscription:

```bash
claude        # then /login once, choosing your Claude subscription
```

There is no API key setting. The launcher runs `claude -p` and removes `ANTHROPIC_API_KEY` from that process's environment, so an exported key is never billed instead. Questions count against your subscription's normal usage limits.

The CLI is started with all tools, MCP servers and session saving turned off: Claude can only reply with text and cannot read files or run commands. To keep answers quick, the CLI is started as soon as you begin typing a question; your text is only sent when you press Enter. The default model is Haiku, the fastest; change it under `[claude]` or in the settings window.

## ⚙️ Configuration

Open the settings window with `settings`, or edit `~/.config/poppi_launcher/config.toml`. The file is created with defaults on first run, every key is optional, and edits are picked up as soon as the file is saved. If it cannot be parsed, the launcher says so, prints the reason to stderr and keeps using the last good values without overwriting your file.

```toml
[theme]
mode = "dark"                 # "dark", "light" or "system"
accent_color = "#4a9eff"
background_color = "#161618"  # dark mode
text_color = "#ececee"
light_background_color = "#f7f7f8"
light_text_color = "#1c1c1e"
border_radius = 14.0          # px
font_size = 16                # pt
width = 640                   # px; height follows the results

[behavior]
close_on_focus_loss = true
stay_resident = true          # keep running in the background for instant startup
max_results = 5               # rows visible at once; longer lists scroll
history_size = 50             # remembered queries, 0 = off

[search]
default_engine = "google"     # google, duckduckgo, youtube or chatgpt
youtube_enabled = true
chatgpt_enabled = true

[calculator]
enabled = true
currency = true               # download ECB exchange rates for currency conversion

[claude]
enabled = true
model = "haiku"               # or "sonnet", "opus", or a full model name
command = "claude"            # the Claude Code executable
# system_prompt = "..."       # instructions for how Claude should answer

[files]
enabled = true
roots = ["~"]                 # directories to index
max_depth = 6                 # levels below each root
include_hidden = false
exclude = ["node_modules", "target", "__pycache__", "venv", "snap"]

[clipboard]
enabled = true
size = 30

[emoji]
skin_tone = "default"         # default, light, medium-light, medium, medium-dark, dark

[aliases]
# keyword = "URL or command"; {} is replaced by what you type after the keyword
gh = "https://github.com/search?q={}"
wiki = "https://en.wikipedia.org/wiki/Special:Search?search={}"
up = "kitty -e sudo dnf upgrade"
```

Colors accept anything GTK CSS understands (`#rrggbb`, `rgba(...)`, names).

**Aliases.** The first word you type is looked up in `[aliases]`. A value starting with `http://` or `https://` is opened in the browser with the rest of your input URL-encoded in place of `{}`, which makes it a custom search engine. Anything else is run as a shell command, with your input inserted (each word quoted) at `{}` or appended. An alias may reuse a built-in keyword such as `yt` to replace it.

**What is stored where.** Query history, launch counts and recent emojis are in `~/.local/state/poppi_launcher/`; exchange rates are cached in `~/.cache/poppi_launcher/`. Set `history_size = 0` to turn history off. Clipboard history is never stored.

## 📋 Requirements

- **Rust**: a recent stable toolchain (developed and tested with 1.92)
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
