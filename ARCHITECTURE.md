# Poppi Launcher Architecture

## Project Structure

```
poppi_launcher/
├── src/
│   ├── main.rs             # CLI flags, GApplication (single instance), config watching
│   ├── query.rs            # Pure parser: search text -> Intent
│   ├── state.rs            # LauncherState: Intent -> list of ResultItem
│   ├── ui/
│   │   ├── mod.rs          # Launcher window: widgets, show/hide, rendering, footer
│   │   ├── keys.rs         # Keyboard handling
│   │   ├── activate.rs     # Running the selected result
│   │   ├── chat.rs         # The conversation with Claude
│   │   └── smoke.rs        # Scripted UI driver (feature "ui-test" only)
│   ├── rows.rs             # Widgets for individual results, footer key hints
│   ├── settings.rs         # Settings window
│   ├── style.rs, style.css # Stylesheet template, palettes, provider
│   ├── config.rs           # TOML configuration
│   ├── matching.rs         # Tiered text matching and highlight positions
│   ├── app_launcher.rs     # Applications and their actions (GIO), scoring, launching
│   ├── usage.rs            # Launch counts and times (frecency)
│   ├── files.rs            # File index, file search, path completion
│   ├── window_switcher.rs  # Window listing and activation
│   ├── calculator.rs       # Expression evaluator
│   ├── units.rs            # Unit conversion, exchange rates
│   ├── claude.rs           # Claude Code CLI session
│   ├── markdown.rs         # Markdown -> Pango markup for answers
│   ├── emoji_picker.rs     # Emoji search and insertion
│   ├── clipboard.rs        # In-memory clipboard history
│   ├── system.rs           # Lock, suspend, log out, restart, power off
│   ├── history.rs          # Past queries (also recent emojis)
│   ├── alias.rs            # User-defined aliases
│   ├── terminal.rs         # Running commands in a terminal emulator
│   ├── search.rs           # Web search engines
│   └── utils.rs            # PATH lookup, clipboard tools, process helpers
├── data/                   # Desktop entry and icon
├── scripts/                # GNOME shortcut helper, UI smoke test
├── packaging/              # AUR and RPM recipes
├── Makefile                # build / test / lint / smoke / install
└── .github/workflows/ci.yml
```

## Data Flow

1. **Input**: the `Entry` emits `changed`.
2. **Parse**: `query::parse(text, options)` returns an `Intent`. It is pure:
   no I/O and nothing is executed while typing.
3. **Resolve**: `LauncherState::update` turns the intent into up to 50
   `ResultItem`s, sets the `Mode` shown in the chip and the text to highlight,
   and reports background work it needs (`Needs`: window list, file index,
   exchange rates, Claude warm-up).
4. **Render**: `Launcher::render` builds rows (`rows.rs`) or the emoji grid,
   caps the scroll area at `max_results` rows and sizes the window to match.
   `update_footer` shows what the keys do for the selected item.
5. **Run**: `Launcher::activate_index` executes the item, then dismisses the
   window. Failures are shown as a row instead of closing silently.

Parse order: questions (a trailing `?`), settings, aliases, `ask`/`ai`,
clipboard, history, window switcher, web search prefixes, emoji, file search,
typed paths, terminal prefixes, conversions, arithmetic, then application
search. While a conversation with Claude is open it takes precedence over all
of these: anything typed is a follow-up.

In application search, apps come first; system actions follow (or lead when
the query is exactly their name), then a `$PATH` command, and web searches
only when nothing else matched.

## Process Model

The application is a single-instance `GtkApplication`. The first process owns
the D-Bus name `com.poppi.launcher`; later invocations forward `activate` to
it and exit, and `ui::activate` toggles the window. `--quit` activates the
exported `quit` action of the running instance.

With `behavior.stay_resident = true` (default) dismissing hides the window and
the process stays alive. With `false` the window is destroyed and the process
exits once no other window (settings) is open.

All state lives on the GTK main thread (`Rc<RefCell<..>>`). Blocking jobs
(listing windows, indexing files, downloading rates) run through
`gio::spawn_blocking`. Claude's output arrives on a channel from a reader
thread and is drained by a GLib timer.

The config file is watched with a `gio::FileMonitor`; a valid change replaces
the shared `Config`, reapplies the stylesheet and refreshes an open launcher.

## Components

### Matching (`matching.rs`)
- Tiers, best first: exact, prefix, word prefix, all query words, initials,
  substring, then compact fuzzy. A fuzzy match must start at a word boundary
  and span at most about twice the query length, so scattered letters never
  match. Shorter texts win within a tier.
- `Matcher::strict` (no initials or fuzzy) is used for descriptions, files
  and system actions.
- `highlight` returns the character positions a query matched, following the
  same preference order; rows emphasise them in the accent colour.

### Applications (`app_launcher.rs`, `usage.rs`)
- `gio::AppInfo::all()` supplies entries: it follows `XDG_DATA_DIRS` (Flatpak,
  Snap), applies user overrides, localizes names and honours
  `NoDisplay`/`Hidden`/`OnlyShowIn`.
- Desktop actions ("New Private Window") are separate entries that rank below
  comparable app matches and launch through `launch_action`.
- Score: name at full value, keywords and generic name one step lower,
  description (strict matches only) lowest, plus a frecency bonus.
- Frecency is `ln(launches + 1)` scaled by how long ago the last launch was.
  It ranks the empty-bar list and breaks ties in search. Stored in
  `$XDG_STATE_HOME/poppi_launcher/usage.json`.

### Files (`files.rs`)
- `build_index` walks the configured roots breadth-first on a worker thread,
  skipping hidden and excluded names and never following directory symlinks,
  capped at 200,000 entries. It is rebuilt in the background when older than
  five minutes; the old index answers meanwhile.
- Ranking: name match tier, shallower paths, recent modification, and how
  often the file was opened from the launcher.
- `complete_path` lists a directory for typed paths, directories first.

### Calculator and conversion (`calculator.rs`, `units.rs`)
- Arithmetic is a dependency-free recursive-descent evaluator.
- `units::parse` recognises `<amount> <unit> in|to|as <unit>`; `convert`
  handles physical units from a table and temperatures by formula.
- Currency codes go through `Rates`: ECB reference rates fetched with `curl`
  from frankfurter.dev, cached in `$XDG_CACHE_HOME` and refreshed when older
  than twelve hours. Nothing is fetched unless a currency conversion is typed.

### Claude (`claude.rs`, `ui/chat.rs`, `markdown.rs`)
- `claude::Session` runs `claude --print --input-format stream-json
  --output-format stream-json --include-partial-messages --model <model>
  --tools "" --strict-mcp-config --disable-slash-commands
  --no-session-persistence` in an empty cache directory. Questions are JSON
  lines on stdin; a reader thread turns stdout into `Delta`/`Done`/`Failed`
  events. One process serves the whole conversation, which is what gives
  follow-ups their context and removes the start-up delay after the first.
- Authentication is the CLI's own login (subscription). `ANTHROPIC_API_KEY`
  and `ANTHROPIC_AUTH_TOKEN` are removed from the child's environment and the
  launcher never reads or stores a key.
- The session starts when a question is being typed (warm-up) and the text is
  sent only on Enter. It is dropped, which kills the process, when the
  conversation is left or the launcher is dismissed.
- Answers are converted from markdown to Pango markup; the result is checked
  with `pango::parse_markup` and shown as plain text if it does not parse.

### Window switcher (`window_switcher.rs`)
- GNOME: the `window-calls` extension over D-Bus (`List`, `Activate`). Its
  stacking order, reversed, gives most-recently-used order.
- Fallback for X11: `wmctrl -l -x` filtered with `xprop`, then `xdotool`.
- Icons come from matching the window class against desktop file ids and
  `StartupWMClass`.

### Clipboard history (`clipboard.rs`)
- Texts are remembered in memory only. They are read on GDK's clipboard
  `changed` signal and whenever the launcher window becomes active; on GNOME
  Wayland the latter is the only time an application may read the clipboard.

### System actions (`system.rs`)
- `loginctl lock-session`, `systemctl suspend`, and `gnome-session-quit` for
  the actions that end the session (GNOME then shows its own confirmation).
  Without GNOME they fall back to `systemctl`/`loginctl` behind a second
  Enter in the launcher.

### Emoji (`emoji_picker.rs`)
- Data from the `emojis` crate, built lazily for the configured skin tone.
- Insertion happens after the launcher is dismissed: `xdotool` on X11,
  `wtype`/`ydotool` on Wayland, clipboard as the fallback.

### Styling (`style.rs`, `style.css`)
- One display-wide `CssProvider`; launcher rules are scoped to
  `.poppi-launcher`. Every colour is derived with `alpha()` from three values
  (background, text, accent), so one template serves dark and light mode.
- `theme.mode` picks the palette; "system" follows GNOME's `color-scheme`
  setting and reacts when it changes.
- The settings window uses the stock GTK theme in the matching mode.

### Configuration (`config.rs`)
- `#[serde(default)]` on every struct: missing keys use defaults, unknown keys
  are ignored, numbers are clamped. Parse errors are reported, not swallowed.

## Testing

- `cargo test`: unit tests for the parser, state resolution, matching and
  highlighting, calculator, units, config, file indexing, history, aliases,
  markdown, Claude output parsing, window-list parsing and the stylesheet.
- `scripts/ui-smoke.sh` (`make smoke`): builds with the `ui-test` feature and
  drives the real window through every mode in a headless mutter session,
  checking the resulting state. `--shots DIR` saves screenshots, `--theme
  light` switches palette, `--claude` adds a real two-turn conversation.
- Two ignored tests touch the real desktop and network:
  `cargo test -- --ignored --nocapture lists_windows` and `... asks_claude`.

## Dependencies

- `gtk4` (0.9), with `glib`, `gio` and `gdk` used through its re-exports
- `fuzzy-matcher`, `emojis`, `serde`, `serde_json`, `toml`, `dirs`, `urlencoding`

## Future Enhancements

- File content search
- Clipboard history on GNOME Wayland beyond what focus allows (needs a shell
  extension or portal support)
- Plugin system
