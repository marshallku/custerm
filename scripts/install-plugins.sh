#!/usr/bin/env bash
# Install first-party plugins into ~/.config/turm/plugins/ with
# binaries symlinked from a build directory. Solves the common
# "service X is not running" startup error caused by turm being
# launched from a desktop entry whose PATH doesn't include the
# built binaries.
#
# Run after `cargo build --release --workspace`.
#
# Usage:
#   ./scripts/install-plugins.sh           # install every example plugin
#   ./scripts/install-plugins.sh todo git  # install just these two
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
EXAMPLES_DIR="$REPO_ROOT/examples/plugins"
TARGET_DIR="$REPO_ROOT/target/release"
PLUGIN_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/turm/plugins"

if [ ! -d "$EXAMPLES_DIR" ]; then
    echo "error: $EXAMPLES_DIR not found — run from a turm checkout" >&2
    exit 1
fi

# Default to every example plugin that has a plugin.toml.
if [ "$#" -eq 0 ]; then
    set -- $(cd "$EXAMPLES_DIR" && find . -mindepth 2 -maxdepth 2 -name plugin.toml -printf '%h\n' | sed 's|^./||' | sort)
fi

mkdir -p "$PLUGIN_DIR"

for name in "$@"; do
    src="$EXAMPLES_DIR/$name"
    dst="$PLUGIN_DIR/$name"
    if [ ! -f "$src/plugin.toml" ]; then
        echo "skip $name: $src/plugin.toml not found"
        continue
    fi
    mkdir -p "$dst"
    # Copy every non-binary file (manifest + panel.html etc).
    # Plugin authors put HTML/CSS/JS alongside plugin.toml; copy
    # them all so the plugin's WebView panel can find its assets.
    find "$src" -maxdepth 1 -type f -exec cp -f {} "$dst/" \;

    # Symlink the binary if it's a [[services]] plugin. Bare-
    # panel plugins (no services entry) don't need a binary.
    exec_name=$(awk '/^\[\[services\]\]/,/^$/' "$src/plugin.toml" \
                | awk -F'=' '/^[[:space:]]*exec[[:space:]]*=/ { gsub(/[[:space:]"]/, "", $2); print $2; exit }')
    if [ -n "$exec_name" ]; then
        bin_src="$TARGET_DIR/$exec_name"
        if [ ! -x "$bin_src" ]; then
            echo "warn  $name: $bin_src not built — run 'cargo build --release -p $exec_name' first" >&2
            continue
        fi
        ln -sf "$bin_src" "$dst/$exec_name"
        echo "ok    $name (binary: $exec_name)"
    else
        echo "ok    $name (panel-only)"
    fi
done

echo
echo "Restart turm so discover_plugins() picks up the changes."
