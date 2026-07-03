#!/usr/bin/env bash
#
# Install (or uninstall) Sudokah into the per-user XDG directories so it shows up
# in the desktop application menu. No root required.
#
#   ./install.sh              build + install for the current user
#   ./install.sh --uninstall  remove it again
#
set -euo pipefail

APP=sudokah
NAME=Sudokah
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

DATA="${XDG_DATA_HOME:-$HOME/.local/share}"
BIN_DIR="$HOME/.local/bin"
APPS_DIR="$DATA/applications"
ICONS_DIR="$DATA/icons/hicolor"
SIZES="32 48 64 128 256"

# Rebuild the menu/icon caches. Each tool is optional and best-effort.
refresh_caches() {
    update-desktop-database "$APPS_DIR" 2>/dev/null || true
    gtk-update-icon-cache -f -t "$ICONS_DIR" 2>/dev/null || true
    for k in kbuildsycoca6 kbuildsycoca5; do
        if command -v "$k" >/dev/null 2>&1; then "$k" >/dev/null 2>&1 || true; break; fi
    done
}

if [ "${1:-}" = "--uninstall" ]; then
    rm -f "$BIN_DIR/$APP" "$APPS_DIR/$APP.desktop" "$ICONS_DIR/scalable/apps/$APP.svg"
    for s in $SIZES; do rm -f "$ICONS_DIR/${s}x${s}/apps/$APP.png"; done
    refresh_caches
    echo "Uninstalled $NAME."
    exit 0
fi

# --- 1. build + install the release binary -----------------------------------
echo "Building release binary (this can take a minute)..."
( cd "$ROOT" && cargo build --release --bin "$APP" )
mkdir -p "$BIN_DIR"
install -m755 "$ROOT/target/release/$APP" "$BIN_DIR/$APP"

# --- 2. icons ----------------------------------------------------------------
# Always install the scalable SVG; add raster sizes if a rasterizer is present,
# otherwise fall back to the bundled 256px PNG.
mkdir -p "$ICONS_DIR/scalable/apps"
cp "$ROOT/assets/icon.svg" "$ICONS_DIR/scalable/apps/$APP.svg"

raster() { # <size> <dest>
    if   command -v rsvg-convert >/dev/null 2>&1; then rsvg-convert -w "$1" -h "$1" "$ROOT/assets/icon.svg" -o "$2"
    elif command -v inkscape     >/dev/null 2>&1; then inkscape "$ROOT/assets/icon.svg" -w "$1" -h "$1" -o "$2" >/dev/null 2>&1
    elif command -v convert      >/dev/null 2>&1; then convert -background none -resize "${1}x${1}" "$ROOT/assets/icon.svg" "$2"
    else return 1; fi
}

if raster 256 /tmp/.sudokah-icon-probe.png 2>/dev/null; then
    rm -f /tmp/.sudokah-icon-probe.png
    for s in $SIZES; do
        mkdir -p "$ICONS_DIR/${s}x${s}/apps"
        raster "$s" "$ICONS_DIR/${s}x${s}/apps/$APP.png"
    done
else
    echo "No SVG rasterizer (rsvg-convert/inkscape/convert); using bundled 256px PNG."
    mkdir -p "$ICONS_DIR/256x256/apps"
    cp "$ROOT/assets/icon.png" "$ICONS_DIR/256x256/apps/$APP.png"
fi

# --- 3. desktop entry --------------------------------------------------------
mkdir -p "$APPS_DIR"
cat > "$APPS_DIR/$APP.desktop" <<DESK
[Desktop Entry]
Type=Application
Name=$NAME
GenericName=Sudoku
Comment=Play and solve Sudoku puzzles
Exec=$BIN_DIR/$APP
Icon=$APP
Terminal=false
Categories=Game;LogicGame;
Keywords=sudoku;puzzle;numbers;
StartupNotify=true
StartupWMClass=$NAME
DESK

refresh_caches

echo "Installed $NAME -> $BIN_DIR/$APP"
echo "Look for \"$NAME\" in your application menu under Games."
case ":$PATH:" in
    *":$BIN_DIR:"*) : ;;
    *) echo "(Note: $BIN_DIR is not on your PATH, so 'sudokah' won't run from a terminal — the menu entry still works.)" ;;
esac
