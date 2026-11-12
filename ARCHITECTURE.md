# Poppi Launcher Architecture

## Project Structure

```
poppi_launcher/
├── src/
│   ├── main.rs             # CLI flags, GApplication (single instance), config watching
│   ├── query.rs            # Pure parser: search text -> Intent
│   ├── state.rs            # LauncherState: Intent -> list of ResultItem
│   ├── ui/
│   │   ├── mod.rs          # Launcher window: widgets, pooling, show/hide, footer
│   │   ├── keys.rs         # Keyboard handling
│   │   ├── activate.rs     # Running the selected result
│   │   ├── chat.rs         # AI conversation lifecycle, streaming, cancellation
│   │   └── smoke.rs        # Scripted UI driver (feature "ui-test" only)
│   ├── ai/
│   │   ├── mod.rs          # Session, Event channel, one-shot job runner
│   │   ├── providers.rs    # Built-in providers, entry resolution, GNOME keyring
│   │   ├── claude_cli.rs   # Persistent Claude Code CLI session
│   │   ├── command.rs      # Gemini CLI, Codex CLI and custom shell commands
│   │   └── http.rs         # OpenAI-compatible and Anthropic SSE streaming via curl
│   ├── rows.rs             # Reusable RowSlot, TranscriptView, code copy, key hints
│   ├── settings.rs         # Settings window
│   ├── model_editor.rs     # AI models editor widget for settings
│   ├── style.rs, style.css # Stylesheet template, palettes, provider
│   ├── config.rs           # TOML configuration and legacy migration
│   ├── matching.rs         # Tiered text matching and highlight positions
│   ├── app_launcher.rs     # Applications and their actions (GIO), scoring, launching
│   ├── usage.rs            # Launch counts and times (frecency)
│   ├── files.rs            # Parallel file index, .gitignore filtering, path completion
│   ├── window_switcher.rs  # Window listing and activation
│   ├── calculator.rs       # Expression evaluator
│   ├── units.rs            # Unit conversion, exchange rates
│   ├── markdown.rs         # Markdown -> structured prose/code blocks + Pango markup
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
   exchange rates, AI warm-up).
4. **Render**: `Launcher::render` updates pooled `RowSlot`s, `TranscriptView`
   or pooled emoji buttons in place, caps the scroll area at `max_results` rows
   and sizes the window to match. `update_footer` updates pooled `HintSlot`s to
   show what the keys do for the selected item.
5. **Run**: `Launcher::activate_index` executes the item, then dismisses the
   window. Failures are shown as a row instead of closing silently.

Parse order: questions (a trailing `?`), settings, aliases, configured model
names, `ask`/`ai`, clipboard, history, window switcher, web search prefixes,
emoji, file search, typed paths, terminal prefixes, conversions, arithmetic,
then application search. While an AI conversation is open it takes precedence
over all of these: anything typed is a follow-up (or `<model> <follow-up>` to
switch models mid-chat).

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

With the `layer-shell` cargo feature, the window becomes a wlr layer-shell
overlay (anchored to the top, exclusive keyboard) when the compositor supports
the protocol; otherwise, as on GNOME, it stays a plain undecorated window.

All state lives on the GTK main thread (`Rc<RefCell<..>>`). Blocking jobs
(listing windows, indexing files, downloading rates, keyring lookups) run
through `gio::spawn_blocking`. AI output arrives on a channel from a reader
thread and is drained by a GLib timer that updates only the active turn's
widgets in place (`sync_streaming_chat`).

The config file is watched with a `gio::FileMonitor`; a valid change replaces
the shared `Config`, invalidates affected caches, reapplies the stylesheet and
refreshes an open launcher. Installed desktop applications are watched via
`gio::AppInfoMonitor`, and indexed root directories are watched with
`gio::FileMonitor`s so changes take effect without polling.

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
- `gio::AppInfo::all()` supplies entries once on build and refreshes on idle
  whenever `gio::AppInfoMonitor` signals `.desktop` changes: it follows
  `XDG_DATA_DIRS` (Flatpak, Snap), applies user overrides, localizes names and
  honours `NoDisplay`/`Hidden`/`OnlyShowIn`.
- Desktop actions ("New Private Window") are separate entries that rank below
  comparable app matches and launch through `launch_action`.
- Score: name at full value, keywords and generic name one step lower,
  description (strict matches only) lowest, plus a frecency bonus.
- Frecency is `ln(launches + 1)` scaled by how long ago the last launch was.
  It ranks the empty-bar list and breaks ties in search. Stored in
  `$XDG_STATE_HOME/poppi_launcher/usage.json`.

### Files (`files.rs`)
- `build_index` walks the configured roots breadth-first, scanning each depth
  level in parallel across worker threads, skipping hidden/excluded names and
  paths matched by `.gitignore` files (`respect_gitignore = true`), never
  following directory symlinks, capped at 200,000 entries.
- Root and top-level directories are watched via `gio::FileMonitor` so newly
  created, renamed or deleted files mark the index stale immediately; otherwise
  it refreshes in the background when older than five minutes while the
  previous index keeps answering.
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

### AI (`src/ai/`, `ui/chat.rs`, `markdown.rs`)
- Three streaming backend families share one `ai::Session` and `Event` channel:
  - `claude_cli`: persistent `claude --print --input-format stream-json
    --output-format stream-json --include-partial-messages --tools ""
    --strict-mcp-config --disable-slash-commands --no-session-persistence`
    process on the user's Claude subscription (`ANTHROPIC_API_KEY` and
    `ANTHROPIC_AUTH_TOKEN` stripped). Pre-warmed while typing.
  - `command`: one-shot CLI processes (`gemini`, `codex` in read-only sandbox,
    or custom shell commands) receiving conversation history in the prompt.
  - `http`: SSE streaming over `curl --config -` (so keys never appear in
    `argv`) for OpenAI-compatible APIs, Anthropic Messages API, and local
    servers (`ollama`, `lmstudio`). Keys resolve from inline config, GNOME
    keyring (`secret-tool`), or environment variables.
- `Session::cancel` (`Esc` or `Ctrl+C`) stops an in-flight turn without closing
  the launcher and swaps the event channel so no stale deltas leak; follow-up
  turns reuse or seamlessly restart the session.
- `markdown::blocks` splits answers into Pango prose blocks (validated with
  `pango::parse_markup`) and raw fenced code blocks rendered in `.chat-code`
  containers with inline copy buttons and `Ctrl+Shift+C` support. `TranscriptView`
  updates streaming blocks in place without rebuilding finished turns or list
  rows.

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
  highlighting, calculator, units, config and legacy migration, parallel file
  indexing and `.gitignore` rules, history, aliases, markdown prose/code blocks,
  AI provider resolution and streaming/cancellation, window-list parsing and the
  stylesheet.
- `scripts/ui-smoke.sh` (`make smoke`): builds with the `ui-test` feature and
  drives the real window through every mode (including an offline streaming AI
  model, code block copy and `Escape` cancellation) in a headless mutter
  session, checking the resulting state. `--shots DIR` saves screenshots,
  `--theme light` switches palette, `--claude` and `--ollama` test live models.
- Three ignored tests touch the real desktop, keyring and network:
  `cargo test -- --ignored --nocapture lists_windows`, `keyring_round_trip` and
  `POPPI_AI_TEST=provider[:model] cargo test -- --ignored --nocapture asks_for_real`.

## Dependencies

- `gtk4` (0.9), with `glib`, `gio` and `gdk` used through its re-exports
- `fuzzy-matcher`, `emojis`, `serde`, `serde_json`, `toml`, `dirs`, `urlencoding`

## Future Enhancements

- File content search
- Clipboard history on GNOME Wayland beyond what focus allows (needs a shell
  extension or portal support)
- Plugin system
