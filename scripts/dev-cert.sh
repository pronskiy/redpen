#!/usr/bin/env bash
#
# Creates a self-signed code-signing identity for local builds.
#
# macOS keys the Accessibility grant to the app's *designated requirement*. Ad-hoc signing
# has no certificate, so that requirement is the binary hash and every rebuild invalidates
# the grant. Signing with a certificate makes the requirement
#
#     identifier "com.pronskiy.redpen" and certificate leaf = H"<cert hash>"
#
# which names no hash of the code at all, so one grant covers every later build signed with
# the same certificate.
#
# This is not an Apple identity and is not trusted by anything: Gatekeeper still sees an
# unnotarized app, and other machines get nothing out of it. codesign signs with an
# untrusted certificate quite happily — the trust chain is Gatekeeper's business, not the
# signature's — so this deliberately does *not* install a trusted root. Run it once.

set -euo pipefail

NAME="${1:-redpen dev}"
KEYCHAIN="$HOME/Library/Keychains/login.keychain-db"

if security find-identity -p codesigning | grep -qF "$NAME"; then
  echo "'$NAME' already exists — nothing to do."
  security find-identity -p codesigning | grep -F "$NAME"
  exit 0
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# A config file rather than -addext, so this works with whichever openssl is first on PATH
# (/usr/bin/openssl is LibreSSL and only grew -addext recently).
cat > "$tmp/openssl.cnf" <<CNF
[ req ]
distinguished_name = dn
prompt             = no
x509_extensions    = ext

[ dn ]
CN = $NAME

[ ext ]
basicConstraints     = critical,CA:false
keyUsage             = critical,digitalSignature
extendedKeyUsage     = critical,codeSigning
subjectKeyIdentifier = hash
CNF

echo "==> generating a 10-year code-signing certificate for '$NAME'"
openssl req -x509 -newkey rsa:2048 -nodes -days 3650 \
  -config "$tmp/openssl.cnf" \
  -keyout "$tmp/key.pem" -out "$tmp/cert.pem" 2>/dev/null

# -legacy because macOS's security(1) cannot read an AES-256 PKCS#12, and a passphrase
# because it rejects an empty-password one with "MAC verification failed".
P12PW=$(openssl rand -hex 16)
openssl pkcs12 -export -legacy \
  -inkey "$tmp/key.pem" -in "$tmp/cert.pem" \
  -out "$tmp/identity.p12" -passout "pass:$P12PW" 2>/dev/null

echo "==> importing into the login keychain"
security import "$tmp/identity.p12" -k "$KEYCHAIN" -P "$P12PW" \
  -T /usr/bin/codesign >/dev/null

# Optional. Without it the first build pops "codesign wants to use a key in your keychain";
# clicking Always Allow there does the same thing permanently, so this is a convenience,
# not a requirement — press Return to skip.
echo
echo "==> optional: authorise codesign to use the key without a dialog."
echo "    Press Return to skip and click 'Always Allow' at the first build instead."
read -r -s -p "    login keychain password: " kcpass; echo
if [ -n "$kcpass" ]; then
  security set-key-partition-list -S apple-tool:,apple:,codesign: \
    -s -k "$kcpass" "$KEYCHAIN" >/dev/null 2>&1 \
    && echo "    done" || echo "    could not set it — you will get the dialog once"
fi
unset kcpass

echo
security find-identity -p codesigning | grep -F "$NAME"
echo
echo "CSSMERR_TP_NOT_TRUSTED next to it is expected and harmless — nothing needs to trust"
echo "this certificate for codesign to sign with it."
echo
echo "Now build with scripts/build-macos.sh instead of 'npm run tauri build'."
