# Poppi Launcher Architecture

## Project Structure

```
poppi_launcher/
├── src/
│   ├── main.rs             # GApplication setup (single instance), startup
│   ├── query.rs            # Pure parser: search text -> Intent
│   ├── state.rs            # LauncherState: Intent -> list of ResultItem
│   ├── ui.rs               # Launcher window, keyboard handling, running results
│   ├── rows.rs             # Widgets for individual results
│   ├── settings.rs         # Settings window
│   ├── style.rs, style.css # Stylesheet template and provider
│   ├── config.rs           # TOML configuration
│   ├── matching.rs         # Tiered text matching shared by all searches
│   ├── app_launcher.rs     # Application discovery (GIO), scoring, launching
│   ├── usage.rs            # Launch counts for ranking
│   ├── files.rs            # File index, file search, path completion
│   ├── history.rs          # Past queries
│   ├── alias.rs            # User-defined aliases
│   ├── claude.rs           # Questions answered through the Claude Code CLI
│   ├── window_switcher.rs  # Window listing and activation
│   ├── calculator.rs       # Expression evaluator
│   ├── emoji_picker.rs     # Emoji search and insertion
│   ├── terminal.rs         # Running commands in a terminal emulator
│   ├── search.rs           # Web search engines
│   └── utils.rs            # PATH lookup, clipboard, process helpers
├── data/                   # Desktop entry and icon
├── scripts/                # GNOME shortcut helper
├── Makefile                # build / test / lint / install
└── .github/workflows/ci.yml
```

## Data Flow

1. **Input**: the `Entry` emits `changed`.
2. **Parse**: `query::parse(text, options)` returns an `Intent`
   (`Ask`, `Apps`, `Files`, `Path`, `Windows`, `Search`, `Emoji`, `Calc`, `Terminal`,
   `Alias`, `History`, `Settings`, `Empty`).
   It is pure: no I/O and nothing is executed while typing.
3. **Resolve**: `LauncherState::update` turns the intent into `Vec<ResultItem>`
   (up to 50) using the loaded apps, usage counts, emoji table, window list or
   file index, and reports which background loads it needs (`Needs`).
4. **Render**: `Launcher::render` builds rows (`rows.rs`) or the emoji grid,
   caps the scroll area at `max_results` rows and sizes the window to match.
5. **Run**: `Launcher::activate_index` executes the item, then dismisses the
   window. Failures are shown as a row instead of closing silently.

Parse order: questions (a trailing `?`), settings, aliases, history, window switcher, web search prefixes,
emoji, file search, typed paths, terminal prefixes, calculator, then
application search. Applications always beat an
unprefixed command of the same name; a `$PATH` command is offered after the app
results, or first when no app matches, followed by web searches.

## Process Model

The application is a single-instance `GtkApplication`. The first process owns
the D-Bus name `com.poppi.launcher`; later invocations forward `activate` to it
and exit, and `ui::activate` toggles the window.

With `behavior.stay_resident = true` (default) dismissing hides the window and
the process stays alive. With `false` the window is destroyed and the process
exits once no other window (settings) is open.

All state lives on the GTK main thread (`Rc<RefCell<..>>`). The two blocking
jobs, listing windows and indexing files, run through `gio::spawn_blocking`
and their results are picked up by a future on the main context.

## Components

### Matching (`matching.rs`)
- Tiers, best first: exact, prefix, word prefix, all query words, initials,
  substring, then compact fuzzy. A fuzzy match must start at a word boundary
  and span at most about twice the query length, so scattered letters never
  match. Shorter texts win within a tier.
- `Matcher::strict` (no initials or fuzzy) is used for descriptions and files.

### Applications (`app_launcher.rs`, `usage.rs`)
- `gio::AppInfo::all()` supplies entries: it follows `XDG_DATA_DIRS` (Flatpak,
  Snap), applies user overrides, localizes names and honours
  `NoDisplay`/`Hidden`/`OnlyShowIn`.
- Score: name at full value, keywords and generic name one step lower,
  description (strict matches only) lowest, plus a logarithmic bonus from
  launch counts stored in `$XDG_STATE_HOME/poppi_launcher/usage.json`.
- An empty query lists the most launched apps.
- Launch through `AppInfo::launch` with a GDK launch context, so field codes,
  `Terminal=true` and focus handover are handled by GIO.

### Files (`files.rs`)
- `build_index` walks the configured roots breadth-first on a worker thread,
  skipping hidden and excluded names and never following directory symlinks,
  capped at 200,000 entries. The index is kept in memory and rebuilt in the
  background when older than five minutes; the old one answers meanwhile.
- `complete_path` lists a directory for typed paths, directories first.

### History and aliases (`history.rs`, `alias.rs`)
- History is a capped, de-duplicated list in
  `$XDG_STATE_HOME/poppi_launcher/history.json`.
- An alias expands to a URL (arguments URL-encoded) or a shell command
  (arguments quoted word by word), so typed text cannot inject shell syntax.

### Claude (`claude.rs`)
- A query ending in `?` becomes `Intent::Ask`. It is only offered as a row;
  the request starts on Enter.
- `claude::start` spawns `claude --print --output-format stream-json
  --include-partial-messages --model <model> --tools "" --strict-mcp-config
  --disable-slash-commands --no-session-persistence -- <question>` in an empty
  cache directory. Authentication is the CLI's own login (subscription);
  `ANTHROPIC_API_KEY` and `ANTHROPIC_AUTH_TOKEN` are removed from the child's
  environment and no key is ever read or stored by the launcher.
- A reader thread parses the JSON lines into `Delta`/`Done`/`Failed` events on
  a channel; a 40 ms GLib timer on the main thread drains it into
  `LauncherState::ask` and re-renders. Dropping the `Request` kills the
  process, which happens when the text changes or the launcher is dismissed.

### Window switcher (`window_switcher.rs`)
- GNOME: the `window-calls` extension over D-Bus (`List`, `Activate`).
- Fallback for X11: `wmctrl -l -x` filtered with `xprop`, then `xdotool`.
- Icons come from matching the window class against desktop file ids and
  `StartupWMClass`.

### Calculator (`calculator.rs`)
- Recursive-descent evaluator with no dependencies.
- A query is only treated as a calculation if it has a digit and an operator
  and evaluates to a finite number; otherwise it stays an app search.

### Emoji (`emoji_picker.rs`)
- Data from the `emojis` crate, built lazily on first use.
- Insertion happens after the launcher is dismissed (typing tools target the
  focused window): `xdotool` on X11, `wtype`/`ydotool` on Wayland, clipboard
  as the fallback.

### Terminal (`terminal.rs`)
- Terminal choice: `$TERMINAL`, `x-terminal-emulator`, GNOME's setting, then a
  list of common emulators. Each has its own argument style.
- Command detection is a file lookup on `$PATH`; no shell is involved.

### Styling (`style.rs`, `style.css`)
- One display-wide `CssProvider`. Every rule is scoped to `.poppi-launcher` or
  `.poppi-settings`. Per-widget providers do not cascade to children in GTK4,
  and GTK CSS has no `!important`, so scoping plus application priority is
  what makes the rules stick.
- `{{tokens}}` are substituted from the config; colors are validated with
  `gdk::RGBA::parse` first. Saving settings reloads the provider live.

### Configuration (`config.rs`)
- `#[serde(default)]` on every struct: missing keys use defaults, unknown keys
  are ignored, numbers are clamped. Parse errors are reported, not swallowed.

## Testing

`cargo test` covers the parser, state resolution, matching tiers, calculator,
config, file indexing and completion, history, aliases, Claude output parsing, window-list parsing,
terminal arguments and the stylesheet. One ignored test
lists real windows (`cargo test -- --ignored --nocapture lists_windows`) and
another sends one real question (`... asks_claude`).

## Dependencies

- `gtk4` (0.9), with `glib`, `gio` and `gdk` used through its re-exports
- `fuzzy-matcher`, `emojis`, `serde`, `serde_json`, `toml`, `dirs`, `urlencoding`

## Future Enhancements

- File content search
- Editing aliases in the settings window
- Plugin system
- Distribution packages (COPR, AUR)
