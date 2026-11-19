#!/bin/bash
# Install the Rust version of Clipboard Reader for this user: build it, copy it to
# ~/.local/bin, and add an app-menu launcher with its icon.
#   rust/scripts/install.sh               install (or update)
#   rust/scripts/install.sh --uninstall   remove it again (settings and voices are kept)
# It sits beside the Python app's "reader" without replacing it; run one at a time.
set -e

RUST_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
NAME="clipboard-reader"
BIN_DIR="$HOME/.local/bin"
APPS_DIR="$HOME/.local/share/applications"
ICON_DIR="$HOME/.local/share/icons/hicolor"
ICON_SIZES=(16 32 48 64 128 256 512)

refresh_caches() {
	update-desktop-database "$APPS_DIR" 2>/dev/null || true
	gtk-update-icon-cache -f -t "$ICON_DIR" 2>/dev/null || true
}

if [ "$1" = "--uninstall" ]; then
	rm -f "$BIN_DIR/$NAME" "$APPS_DIR/$NAME.desktop"
	for size in "${ICON_SIZES[@]}"; do
		rm -f "$ICON_DIR/${size}x${size}/apps/$NAME.png"
	done
	refresh_caches
	echo "Removed. Your settings and voices in ~/.textReader are still there (the Python app uses them too)."
	exit 0
fi

# Rust: from PATH, or rustup's default place
CARGO="$(command -v cargo || true)"
[ -z "$CARGO" ] && [ -x "$HOME/.cargo/bin/cargo" ] && CARGO="$HOME/.cargo/bin/cargo"
if [ -z "$CARGO" ]; then
	echo "Rust isn't installed. Install it with rustup (no sudo needed), then run this again:"
	echo "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
	exit 1
fi

# What the app needs at run time (it still starts without the optional ones)
missing=()
have_lib() { ldconfig -p 2>/dev/null | grep -q "$1"; }
have_lib "libmpv.so.2" || missing+=("libmpv2 (needed: plays the audio)")
have_lib "libespeak-ng.so.1" || missing+=("libespeak-ng1 (needed by Piper voices)")
command -v xclip >/dev/null || missing+=("xclip (needed: reads the selection and clipboard)")
command -v espeak-ng >/dev/null || missing+=("espeak-ng (optional: the eSpeak engine)")
command -v ffmpeg >/dev/null || missing+=("ffmpeg (optional: more accurate word highlighting)")
if [ ${#missing[@]} -gt 0 ]; then
	echo "⚠️  Not found on this computer:"
	printf '     %s\n' "${missing[@]}"
	echo "   On Debian/Ubuntu/Mint: sudo apt install libmpv2 libespeak-ng1 xclip espeak-ng ffmpeg"
	echo
fi

echo "Building (the first build takes about 6 minutes)..."
(cd "$RUST_DIR" && "$CARGO" build --release)
BUILT="$RUST_DIR/target/release/$NAME"

echo "Installing to $BIN_DIR/$NAME..."
mkdir -p "$BIN_DIR"
# Copy then rename, so this works even while the app is running
cp "$BUILT" "$BIN_DIR/.$NAME.new"
chmod +x "$BIN_DIR/.$NAME.new"
mv -f "$BIN_DIR/.$NAME.new" "$BIN_DIR/$NAME"

echo "Adding the icon..."
"$BIN_DIR/$NAME" --write-icons "$ICON_DIR"

echo "Adding the app-menu launcher..."
mkdir -p "$APPS_DIR"
cat > "$APPS_DIR/$NAME.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Clipboard Reader (Rust)
Comment=Read selected text or the clipboard aloud
Exec=$BIN_DIR/$NAME
Icon=$NAME
StartupWMClass=$NAME
Terminal=false
Categories=Utility;Accessibility;
EOF
chmod +x "$APPS_DIR/$NAME.desktop"
refresh_caches

echo
echo "Installed $("$BIN_DIR/$NAME" --version)."
echo "Start it from the app menu (Clipboard Reader (Rust)) or with: $NAME"
case ":$PATH:" in *":$BIN_DIR:"*) ;; *) echo "($BIN_DIR isn't on your PATH, so use the full path: $BIN_DIR/$NAME)";; esac
if pgrep -x reader >/dev/null; then
	echo "Note: the Python version (reader) is running. Quit it before starting this one, or both will answer the shortcuts."
fi
