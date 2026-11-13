#!/usr/bin/env bash
# Drive the real launcher UI through its main modes in a headless compositor
# and check what it shows. Needs mutter and dbus-run-session.
#
#   scripts/ui-smoke.sh [--shots DIR] [--theme dark|light] [--claude] [--ollama]
#
# --shots DIR  also save a screenshot of every step into DIR
# --theme MODE run in dark (default) or light mode
# --claude     additionally ask Claude Haiku one question and a follow-up
#              (uses the logged-in Claude Code account; needs network access)
# --ollama     with --claude: then send a follow-up to the local llama3.2
#              through Ollama, which must be running
#
# Nothing in your own config, history or usage data is touched: the launcher
# runs with temporary XDG directories.
set -euo pipefail

shots=""
theme="dark"
with_claude=0
with_ollama=0
while [ $# -gt 0 ]; do
    case "$1" in
        --shots) shots=$(realpath -m "$2"); shift 2 ;;
        --theme) theme=$2; shift 2 ;;
        --claude) with_claude=1; shift ;;
        --ollama) with_ollama=1; shift ;;
        *) echo "usage: $0 [--shots DIR] [--theme dark|light] [--claude] [--ollama]" >&2; exit 2 ;;
    esac
done

for tool in mutter dbus-run-session; do
    command -v "$tool" >/dev/null || { echo "missing: $tool" >&2; exit 2; }
done

repo=$(cd "$(dirname "$0")/.." && pwd)
cd "$repo"
cargo build --quiet --features ui-test
bin="$repo/target/debug/poppi_launcher"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
home="$work/home"
mkdir -p "$work/config/poppi_launcher/scripts" "$work/state/poppi_launcher" "$work/cache" \
    "$home/Documents/reports" "$home/Pictures"
touch "$home/Documents/report-2026.pdf" "$home/Documents/reports/annual report.txt" \
    "$home/Pictures/report-cover.png" "$home/deploy.sh"
printf 'Meeting notes\nToken: PoppiSecret\n' > "$home/Documents/notes.md"
cat > "$work/config/poppi_launcher/scripts/greet.sh" <<'SCRIPT'
#!/usr/bin/env bash
# @poppi.keyword greet
# @poppi.name Greeter
# @poppi.mode list
printf 'Hello %s\tScript row\n' "${1:-world}"
SCRIPT
chmod +x "$work/config/poppi_launcher/scripts/greet.sh"
[ -n "$shots" ] && mkdir -p "$shots"

cat > "$work/config/poppi_launcher/config.toml" <<CONFIG
[theme]
mode = "$theme"

[files]
roots = ["$home"]

[aliases]
gh = "https://github.com/search?q={}"

[voice]
# Stands in for whisper-cli; POPPI_UI_RECORDER stands in for the microphone.
command = "echo ' Lock screen.'"

[ai]
default = "haiku"

[ai.models.haiku]
provider = "claude"
model = "haiku"

[ai.models.llama]
provider = "ollama"
model = "llama3.2"

[ai.models.gpt]
provider = "openai"
model = "gpt-5-mini"

[ai.models.mock]
provider = "command"
command = "input=\$(cat); case \"\$input\" in *hang*) echo 'Partial answer'; sleep 30 ;; *xyzzy*) printf 'Opening it.\\n\\n[[open: ~/Downloads]]\\n[[system: lock]]\\n' ;; *) printf 'Use tar:\\n\\n\`\`\`sh\\ntar -xzf archive.tar.gz\\n\`\`\`\\n' ;; esac"
CONFIG
echo '["yt rust gtk4 tutorial","2^10"]' > "$work/state/poppi_launcher/history.json"

steps=(
    ""                      #  0
    "zzqqnothing"           #  1
    "2+2*3"                 #  2
    "5 km in miles"         #  3
    "f report"              #  4
    "<wait>"                #  5
    "$home/Doc"             #  6
    "<tab>"                 #  7
    ":hea"                  #  8
    "<down:5>"              #  9
    "gh gtk4 rust"          # 10
    "history"               # 11
    "lock"                  # 12
    "clip"                  # 13
    "sw"                    # 14
    "<alt-up>"              # 15
)
step_ms=700
if [ "$with_claude" = 1 ]; then
    steps+=(
        "what is the capital of Slovenia?"   # 16
        "<enter>"                            # 17
        "<wait>" "<wait>" "<wait>"           # 18-20
        "and of Croatia"                     # 21
        "<enter>"                            # 22
        "<wait>" "<wait>" "<wait>"           # 23-25
    )
    if [ "$with_ollama" = 1 ]; then
        steps+=(
            "llama what was the first country I asked about"   # 26
            "<enter>"                                          # 27
            "<wait>" "<wait>" "<wait>" "<wait>" "<wait>"       # 28-32
        )
    fi
    steps+=("<backspace>")
    step_ms=2000
else
    steps+=(
        "mock show tar command"              # 16
        "<enter>"                            # 17
        "<ctrl-shift-c>"                     # 18
        "hang"                               # 19
        "<enter>"                            # 20
        "<escape>"                           # 21
        "<backspace>"                        # 22
        "#4a9eff"                            # 23
        "<ctrl-k>"                           # 24
        "<escape>"                           # 25
        "3pm est in cet"                     # 26
        "greet Poppi"                        # 27
        "<wait>"                             # 28
        "g PoppiSecret"                      # 29
        "<wait>"                             # 30
        ""                                   # 31
        "<ctrl-d>"                           # 32
        "<key-enter>"                        # 33
        "<wait>"                             # 34
        "<ctrl-d>"                           # 35
        "<escape>"                           # 36
        "mock xyzzy"                         # 37
        "<enter>"                            # 38
        "<wait>"                             # 39
        "<backspace>"                        # 40
    )
fi
steps+=("setup" "<enter>" "<wait>" "poppi settings" "<enter>")
script=$(IFS='|'; echo "${steps[*]}")

log="$work/ui.log"
cat > "$work/session.sh" <<SESSION
#!/usr/bin/env bash
mutter --headless --wayland --no-x11 --virtual-monitor 1280x900 \
    --wayland-display poppi-ui-test >"$work/mutter.log" 2>&1 &
compositor=\$!
sleep 2
WAYLAND_DISPLAY=poppi-ui-test DISPLAY= GTK_A11Y=none \
XDG_CONFIG_HOME="$work/config" XDG_STATE_HOME="$work/state" XDG_CACHE_HOME="$work/cache" \
POPPI_UI_RECORDER="head -c 64000 /dev/zero; exec sleep 30" \
POPPI_UI_SCRIPT='$script' POPPI_UI_SHOTS='$shots' POPPI_UI_STEP_MS=$step_ms \
    timeout 120 "$bin" >"$log" 2>&1
status=\$?
kill \$compositor 2>/dev/null
exit \$status
SESSION
chmod +x "$work/session.sh"
dbus-run-session -- "$work/session.sh" >/dev/null 2>&1 || true

failures=0
expect() { # step-number  description  extended-regex
    local line
    line=$(grep -E "^UI-TEST step=$1 " "$log" || true)
    if [ -n "$line" ] && grep -Eq -- "$3" <<<"$line"; then
        echo "ok    $2"
    else
        echo "FAIL  $2"
        echo "      expected /$3/ in: ${line:-<no output for step $1>}"
        failures=$((failures + 1))
    fi
}

expect 0  "starts empty and visible"            'mode="" results=0 .*visible=true'
expect 1  "unknown text offers web searches"    'results=3 first=search selected=search'
expect 2  "arithmetic"                          'mode="Calculator" results=1 first=calc'
expect 3  "unit conversion"                     'mode="Calculator" results=1 first=calc'
expect 5  "file search finds indexed files"     'mode="Files" results=4 first=file'
expect 6  "typed path lists the directory"      'mode="Files" results=1 first=file'
expect 7  "Tab completes the path"              'query="[^"]*/Documents/" .*results=3'
expect 8  "emoji grid"                          'mode="Emoji" results=[1-9][0-9]+ first=emoji'
expect 9  "emoji grid scrolls with selection"   'scroll=[1-9][0-9]*/'
expect 10 "alias expands"                       'mode="Alias" results=1 first=alias'
expect 11 "history lists past queries"          'mode="History" results=2 first=history'
expect 12 "system action leads on exact name"   'first=system selected=system'
expect 13 "clipboard mode"                      'mode="Clipboard"'
expect 14 "window mode"                         'mode="Windows"'
expect 15 "Alt+Up recalls the last query"       'query="yt rust gtk4 tutorial"'
last=$(( ${#steps[@]} - 1 ))
if [ "$with_claude" = 1 ]; then
    expect 16 "a question offers the default model" 'mode="AI" results=1 first=ask'
    expect 20 "Claude answers"                    'first=transcript .*chat=haiku:Done:[1-9]'
    expect 21 "typing continues the conversation" 'results=2 first=transcript selected=ask'
    expect 25 "the follow-up is answered"         'query="" .*chat=haiku:Done:[1-9]'
    if [ "$with_ollama" = 1 ]; then
        expect 26 "a model name switches the model" 'results=2 first=transcript selected=ask'
        expect 32 "the other model answers"         'chat=llama:Done:[1-9]'
    fi
    expect "$((last - 2))" "Backspace leaves the conversation" 'mode="" results=0 .*chat=-'
else
    expect 16 "model prefix selects that AI model"     'mode="AI" results=1 first=ask'
    expect 17 "AI streams markdown and code block"     'first=transcript .*chat=mock:Done:[1-9]'
    expect 18 "Ctrl+Shift+C copies code in place"      'first=transcript .*chat=mock:Done:[1-9][0-9]* .*visible=true'
    expect 19 "typing continues the conversation"      'results=2 first=transcript selected=ask'
    expect 20 "follow-up starts streaming"             'first=transcript .*chat=mock:(Thinking|Answering):'
    expect 21 "Escape stops answer without closing"    'first=transcript .*chat=mock:(Done|Failed):[1-9][0-9]* .*visible=true'
    expect 22 "Backspace leaves the conversation"      'mode="" results=0 .*chat=-'
    expect 23 "color swatch conversion"                'mode="Calculator" results=1 first=calc'
    expect 24 "Ctrl+K opens result action menu"        'mode="Actions" results=2 first=action'
    expect 25 "Escape leaves action menu to query"     'query="#4a9eff" mode="Calculator" results=1'
    expect 26 "timezone conversion"                    'mode="Calculator" results=1 first=calc'
    expect 28 "custom list script returns rows"        'mode="Script" results=1 first=script-item'
    expect 30 "file content search finds line"         'mode="Files" results=1 first=grep'
    expect 32 "Ctrl+D starts listening"                'voice=recording'
    expect 34 "dictated text lands in the search bar"  'query="Lock screen" .*first=system .*voice=idle'
    expect 35 "Ctrl+D listens again"                   'voice=recording'
    expect 36 "Escape cancels without closing"         'query="Lock screen" .*visible=true .*voice=idle'
    expect 39 "AI actions become rows to run"          'first=transcript selected=proposed .*chat=mock:Done'
    expect 39 "an action and a system action proposed" 'results=3 '
fi
expect $((last - 4)) "typing setup offers the setup"      'mode="Apps" results=1 first=setup'
expect $((last - 3)) "opening the setup hides the launcher" 'visible=false'
expect $((last - 1)) "settings entry"            'mode="Settings" results=1 first=settings'
expect "$last" "opening settings hides the launcher" 'visible=false'
if grep -q "^UI-TEST done" "$log"; then
    echo "ok    script ran to the end"
else
    echo "FAIL  script did not finish"; failures=$((failures + 1))
fi
if grep -Eiq "panicked|Gtk-CRITICAL|Theme parser (error|warning)" "$log"; then
    echo "FAIL  errors in the log:"; grep -Ei "panicked|Gtk-CRITICAL|Theme parser" "$log" | head -5
    failures=$((failures + 1))
else
    echo "ok    no panics, GTK criticals or CSS errors"
fi

if [ "$failures" -gt 0 ]; then
    echo; echo "$failures check(s) failed. Full log:"; grep "^UI-TEST" "$log" || cat "$log"
    exit 1
fi
echo; echo "All UI checks passed."
