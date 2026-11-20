#!/bin/bash
# Pack the release build into dist/clipboard-reader-linux-x86_64.tar.gz for downloading:
# the program, install.sh and the README. Run after `cargo build --release`
# (the GitHub workflow in .github/workflows/build.yml does both).
set -e
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
NAME="clipboard-reader-linux-x86_64"
OUT="$ROOT/dist"

rm -rf "$OUT/$NAME" "$OUT/$NAME.tar.gz" "$OUT/$NAME.tar.gz.sha256"
mkdir -p "$OUT/$NAME"
cp "$ROOT/target/release/clipboard-reader" "$ROOT/scripts/install.sh" "$ROOT/README.md" "$OUT/$NAME/"
chmod +x "$OUT/$NAME/clipboard-reader" "$OUT/$NAME/install.sh"
tar -C "$OUT" -czf "$OUT/$NAME.tar.gz" "$NAME"
(cd "$OUT" && sha256sum "$NAME.tar.gz" > "$NAME.tar.gz.sha256")
echo "Packed $OUT/$NAME.tar.gz ($(du -h "$OUT/$NAME.tar.gz" | cut -f1))"
