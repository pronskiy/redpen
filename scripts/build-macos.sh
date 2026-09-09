#!/usr/bin/env bash
#
# Builds the app, signed with the local dev identity when there is one.
#
# tauri.conf.json pins `signingIdentity: "-"` so a plain `tauri build` at least seals the
# bundle. Sealed is not the same as stable: an ad-hoc signature has no certificate, so its
# designated requirement is the binary hash and this build's Accessibility grant dies with
# the next build. `scripts/dev-cert.sh` fixes that, and this picks the identity up when it
# is there. APPLE_SIGNING_IDENTITY overrides the value in tauri.conf.json — verified, not
# assumed — which is the same lever release.yml pulls in CI.

set -euo pipefail

IDENTITY="${REDPEN_SIGNING_IDENTITY:-redpen dev}"

if security find-identity -p codesigning | grep -qF "$IDENTITY"; then
  echo "==> signing with '$IDENTITY' — the Accessibility grant survives this rebuild"
  APPLE_SIGNING_IDENTITY="$IDENTITY" exec npm run tauri -- build "$@"
fi

cat >&2 <<MSG
==> no '$IDENTITY' code-signing identity; falling back to ad-hoc.

    This build gets a fresh binary hash, so macOS will drop the Accessibility grant and
    ⌥⌘E will go quiet until you grant it again. Run scripts/dev-cert.sh once to stop that
    happening on every build.

MSG
exec npm run tauri -- build "$@"
