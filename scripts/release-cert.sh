#!/usr/bin/env bash
#
# Generates the certificate that CI signs releases with, and the two GitHub secrets that
# carry it.
#
# Why a certificate for releases at all: macOS keys the Accessibility grant to the app's
# designated requirement. Ad-hoc puts the binary hash there, so every update a user
# installs revokes their grant and ⌥⌘E goes quiet until they re-grant. Signing with a
# certificate makes the requirement name the certificate instead, and one grant then
# covers every future release.
#
#   ⚠ THIS CERTIFICATE IS FOREVER. Regenerating it changes the designated requirement, and
#   every existing user loses their grant on the next update. Generate it once, keep the
#   secrets, and do not rotate it without a reason worth that cost.
#
# It is not an Apple identity: Gatekeeper still blocks the first launch, and the README's
# quarantine instructions still apply. Only a Developer ID fixes that.

set -euo pipefail

NAME="${1:-redpen release}"
OUT=$(mktemp -d)

cat > "$OUT/openssl.cnf" <<CNF
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

# 100 years: an expired certificate would break signing, and nothing here checks validity
# dates anyway — TCC matches the certificate's hash, not its lifetime.
openssl req -x509 -newkey rsa:2048 -nodes -days 36500 \
  -config "$OUT/openssl.cnf" \
  -keyout "$OUT/key.pem" -out "$OUT/cert.pem" 2>/dev/null

PW=$(openssl rand -hex 24)
openssl pkcs12 -export -legacy \
  -inkey "$OUT/key.pem" -in "$OUT/cert.pem" \
  -out "$OUT/identity.p12" -passout "pass:$PW" 2>/dev/null

base64 < "$OUT/identity.p12" > "$OUT/p12.b64"
printf '%s' "$PW" > "$OUT/password.txt"
rm -f "$OUT/key.pem" "$OUT/identity.p12" "$OUT/openssl.cnf"
chmod 600 "$OUT"/*

# The fingerprint that ends up inside every release's designated requirement. Worth
# recording: it is how you tell later whether a build came from this certificate.
FPR=$(openssl x509 -in "$OUT/cert.pem" -noout -fingerprint -sha1 \
      | cut -d= -f2 | tr -d ': ')

cat <<MSG

Generated '$NAME'
  certificate SHA-1: $FPR
  designated requirement releases will carry:
      identifier "com.pronskiy.redpen" and certificate leaf = H"$(echo "$FPR" | tr 'A-Z' 'a-z')"

Secrets are in $OUT (outside the repo, mode 600). Upload them:

  gh secret set MACOS_CERT_P12      --repo pronskiy/redpen < "$OUT/p12.b64"
  gh secret set MACOS_CERT_PASSWORD --repo pronskiy/redpen < "$OUT/password.txt"

Then delete them:

  rm -rf "$OUT"

Keep a copy somewhere safe first if you want to be able to sign outside CI — losing the
key means the next release gets a new requirement and every user re-grants once.
MSG
