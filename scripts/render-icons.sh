#!/usr/bin/env bash
# Render src-tauri/icons/* from the SVG sources in assets/ (DESIGN.md §6 logo spec).
# Needs: rsvg-convert, magick, iconutil (macOS), npx (@tauri-apps/cli).
set -euo pipefail
cd "$(dirname "$0")/.."

SQ=assets/contractorcrm-icon-square.svg     # rx 0 — Windows/Linux/store tiles
RND=assets/contractorcrm-icon.svg           # rx 22% — macOS tile
S16=assets/contractorcrm-icon-16.svg        # 16px two-column variant, square
S16MAC=assets/contractorcrm-icon-16-macos.svg
OUT=src-tauri/icons
TMP=$(mktemp -d)

# 1. Full cross-platform set from the square master via tauri icon.
rsvg-convert -w 1024 -h 1024 "$SQ" -o "$TMP/sq-1024.png"
npx tauri icon "$TMP/sq-1024.png" -o "$OUT" >/dev/null

# 2. macOS icns from the rounded tile; 16px drops to the two-column mark.
ISET="$TMP/icon.iconset"
mkdir -p "$ISET"
rsvg-convert -w 16   -h 16   "$S16MAC" -o "$ISET/icon_16x16.png"
for s in 32 128 256 512; do
  rsvg-convert -w $s -h $s "$RND" -o "$ISET/icon_${s}x${s}.png"
done
cp "$ISET/icon_32x32.png"    "$ISET/icon_16x16@2x.png"
rsvg-convert -w 64   -h 64   "$RND" -o "$ISET/icon_32x32@2x.png"
cp "$ISET/icon_256x256.png"  "$ISET/icon_128x128@2x.png"
cp "$ISET/icon_512x512.png"  "$ISET/icon_256x256@2x.png"
rsvg-convert -w 1024 -h 1024 "$RND" -o "$ISET/icon_512x512@2x.png"
iconutil -c icns "$ISET" -o "$OUT/icon.icns"

# 3. Windows ico: 16px layer uses the two-column variant, rest the full square mark.
rsvg-convert -w 16 -h 16 "$S16" -o "$TMP/ico-16.png"
for s in 24 32 48 64 128 256; do
  rsvg-convert -w $s -h $s "$SQ" -o "$TMP/ico-$s.png"
done
magick "$TMP/ico-16.png" "$TMP/ico-24.png" "$TMP/ico-32.png" "$TMP/ico-48.png" \
       "$TMP/ico-64.png" "$TMP/ico-128.png" "$TMP/ico-256.png" "$OUT/icon.ico"

rm -rf "$TMP"
echo "Icons rendered into $OUT"
