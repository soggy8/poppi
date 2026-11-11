#!/usr/bin/env bash
# Bind a key to poppi_launcher as a GNOME custom shortcut.
#
#   scripts/setup-gnome-shortcut.sh '<Control>space'
#
# Existing custom shortcuts are kept; a new slot is appended. Run it again
# with a different binding to change the key.
set -euo pipefail

if [ $# -ne 1 ]; then
    echo "usage: $0 '<Modifier>key'   e.g. '<Control>space' or '<Super>d'" >&2
    exit 1
fi

binding=$1
command=${POPPI_COMMAND:-poppi_launcher}
schema=org.gnome.settings-daemon.plugins.media-keys
base=/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings
entry_schema=$schema.custom-keybinding

if ! command -v gsettings >/dev/null; then
    echo "gsettings not found; this script only works on GNOME." >&2
    exit 1
fi

existing=$(gsettings get $schema custom-keybindings)
paths=$(printf '%s' "$existing" | grep -o "'[^']*'" | tr -d "'" || true)

# Reuse our slot if we already created one.
target=""
for path in $paths; do
    if [ "$(gsettings get $entry_schema:"$path" name)" = "'Poppi Launcher'" ]; then
        target=$path
    fi
done

if [ -z "$target" ]; then
    n=0
    while printf '%s\n' $paths | grep -qx "$base/custom$n/"; do
        n=$((n + 1))
    done
    target="$base/custom$n/"
    list="["
    for path in $paths; do
        list+="'$path', "
    done
    list+="'$target']"
    gsettings set $schema custom-keybindings "$list"
fi

gsettings set $entry_schema:"$target" name 'Poppi Launcher'
gsettings set $entry_schema:"$target" command "$command"
gsettings set $entry_schema:"$target" binding "$binding"

echo "Bound $binding to '$command'."
