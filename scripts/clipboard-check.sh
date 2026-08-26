#!/usr/bin/env bash
#
# Semi-automates the "clipboard integrity" half of guardrail A1 (SPEC step A1.4).
#
# You still have to select text and press ⌥⌘E — nothing can automate that. This handles
# the fiddly part: seeding a known canary onto the pasteboard and checking afterwards
# whether it actually survived the capture.
#
#   scripts/clipboard-check.sh text     # plain text canary
#   scripts/clipboard-check.sh image    # PNG — the "all types, not just text" claim
#   scripts/clipboard-check.sh rtf      # rich text
#
set -uo pipefail
MODE="${1:-text}"
CANARY="CANARY-do-not-lose-me-$$"
PNG="src-tauri/icons/32x32.png"

info() { osascript -e 'clipboard info' 2>/dev/null; }

case "$MODE" in
  text)
    printf '%s' "$CANARY" | pbcopy
    expect="the exact text \"$CANARY\""
    ;;
  image)
    [ -f "$PNG" ] || { echo "missing $PNG" >&2; exit 1; }
    osascript -e "set the clipboard to (read (POSIX file \"$PWD/$PNG\") as «class PNGf»)" >/dev/null \
      || { echo "could not put a PNG on the pasteboard" >&2; exit 1; }
    expect="a PNG image"
    ;;
  rtf)
    osascript -e 'set the clipboard to (("'"$CANARY"'") as «class RTF »)' >/dev/null 2>&1 \
      || { echo "could not put RTF on the pasteboard; copy from Pages or Mail by hand instead" >&2; exit 1; }
    expect="rich text"
    ;;
  *) echo "usage: $0 [text|image|rtf]" >&2; exit 2 ;;
esac

echo "seeded: $expect"
echo "types now: $(info)"
echo
echo "→ Now select some text in another app and press ⌥⌘E."
echo "→ Then come back here and press Enter."
read -r _

after="$(info)"
echo "types after: $after"
echo

case "$MODE" in
  text)
    got="$(pbpaste)"
    if [ "$got" = "$CANARY" ]; then echo "PASS — canary survived the capture"
    else echo "FAIL — clipboard now holds: ${got:0:60}"; fi
    ;;
  image)
    if echo "$after" | grep -qE 'PNGf|TIFF'; then echo "PASS — image survived the capture"
    else echo "FAIL — the image is gone"; fi
    ;;
  rtf)
    if echo "$after" | grep -q 'RTF'; then echo "PASS — rich text survived the capture"
    else echo "FAIL — the RTF is gone"; fi
    ;;
esac

echo
echo "NOTE: if a clipboard manager (Raycast, Maccy) is running, a FAIL may be correct"
echo "behaviour — redpen deliberately skips the restore when changeCount != before + 1"
echo "rather than fighting it. Check the manager's history before filing a bug."
