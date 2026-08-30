#!/usr/bin/env bash
#
# Regenerates every icon asset from the two SVG sources in design/.
#
#   scripts/build-icons.sh            # rebuild all icon assets
#   scripts/build-icons.sh --sheet    # also write a contact sheet to design/.out/ for eyeballing
#
# There are two surfaces and they are NOT the same artwork:
#
#   design/appicon.svg       -> src-tauri/icons/*  (bundle icon: Finder, Spotlight, About)
#   design/tray-template.svg -> src-tauri/icons/tray-template.png  (the menu bar)
#
# The menu-bar glyph is a macOS *template image*: pure black plus alpha, which the system
# recolours for the light and dark menu bar. Colour art there renders as-is and looks wrong in
# both, which is why the app icon cannot simply be reused — see src-tauri/src/lib.rs.
#
# redpen runs under ActivationPolicy::Accessory, so there is no Dock icon. The menu-bar glyph is
# the mark you actually see all day; the bundle icon shows up in Finder, Spotlight and About.
#
# Requires: rsvg-convert (librsvg), and npx for the Tauri CLI.

set -euo pipefail
cd "$(dirname "$0")/.."

command -v rsvg-convert >/dev/null || { echo "need rsvg-convert: brew install librsvg" >&2; exit 1; }

OUT=design/.out
mkdir -p "$OUT"

echo "==> app icon"
rsvg-convert -w 1024 -h 1024 design/appicon.svg -o "$OUT/appicon-1024.png"
# Regenerates 32x32, 128x128, 128x128@2x, icon.icns, icon.ico and the Windows Store PNGs listed
# in src-tauri/tauri.conf.json. Output defaults to the icons/ dir beside tauri.conf.json.
npx --no-install tauri icon "$OUT/appicon-1024.png"

# The CLI also emits full iOS and Android asset trees. redpen is macOS-only (accessory policy,
# NSPanel, AXIsProcessTrusted), so those are ~40 files of noise — drop them rather than commit them.
rm -rf src-tauri/icons/ios src-tauri/icons/android

echo "==> menu-bar template"
# 36px tall is deliberate, not arbitrary: tray-icon forces the NSImage to 18pt
# (tray-icon/src/platform_impl/macos/mod.rs), so an exact 2x avoids retina resampling.
rsvg-convert -w 38 -h 36 design/tray-template.svg -o src-tauri/icons/tray-template.png

if [ "${1:-}" = "--sheet" ]; then
  echo "==> contact sheet"
  command -v magick >/dev/null || { echo "need magick for --sheet: brew install imagemagick" >&2; exit 1; }
  for bg in "E8E8EA" "1E1E20"; do
    rm -f "$OUT"/_r_*.png
    for s in 512 256 128 64 32 16; do
      magick "$OUT/appicon-1024.png" -resize ${s}x${s} -background "#$bg" \
        -gravity center -extent 540x540 "$OUT/_r_$(printf '%04d' $((1000-s))).png"
    done
    magick "$OUT"/_r_*.png +append -background "#$bg" -bordercolor "#$bg" -border 20 "$OUT/row-$bg.png"
  done
  magick "$OUT/row-E8E8EA.png" "$OUT/row-1E1E20.png" -append -resize 1800x "$OUT/contact-sheet.png"
  rm -f "$OUT"/_r_*.png "$OUT"/row-*.png
  echo "    $OUT/contact-sheet.png"
fi

echo "done."
