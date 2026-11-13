#!/usr/bin/env bash
# Signs release files for the Linux server (SRV-201): for each file, an
# ed25519 signature over the whole file (<file>.sig, 64 raw bytes) and its
# SHA-256 (<file>.sha256, as sha256sum prints it).
#
#   NODUS_SERVER_SIGNING_KEY=<private key PEM> packaging/server/sign.sh <file>...
#
# The key comes from the repo's secret of that name, which
# scripts/server-signing-key.sh sets. Before signing anything it checks that
# the public key written into install-server.sh (next to this script) is
# that key's, so a release can't ship files its own install script refuses.
# The key is never printed, and its copy on disk goes when this ends.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
error() {
	echo "::error::$*" >&2
	exit 1
}

[ $# -ge 1 ] || { echo "usage: NODUS_SERVER_SIGNING_KEY=… sign.sh <file>..." >&2; exit 2; }
if [ -z "${NODUS_SERVER_SIGNING_KEY:-}" ]; then
	error "NODUS_SERVER_SIGNING_KEY isn't set: the Linux server can't be signed. Run scripts/server-signing-key.sh once to make the key and set the secret."
fi

umask 077
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
printf '%s\n' "$NODUS_SERVER_SIGNING_KEY" >"$work/key.pem"
openssl pkey -in "$work/key.pem" -text -noout 2>/dev/null | grep -q '^ED25519 Private-Key' ||
	error "NODUS_SERVER_SIGNING_KEY isn't an ed25519 private key in PEM form."
openssl pkey -in "$work/key.pem" -pubout -out "$work/public.pem"

# What install-server.sh will check against: its SIGNING_KEY, as bash reads it.
embedded=$(bash -c "$(sed -n '/^# BEGIN SIGNING KEY$/,/^# END SIGNING KEY$/p' "$here/install-server.sh")"$'\nprintf "%s\\n" "$SIGNING_KEY"')
if [ -z "$embedded" ]; then
	error "install-server.sh has no signing key yet (the TODO placeholder). Run scripts/server-signing-key.sh and commit the script."
fi
if [ "$embedded" != "$(cat "$work/public.pem")" ]; then
	error "The public key in install-server.sh isn't NODUS_SERVER_SIGNING_KEY's: servers would refuse these files. Write the right one in with scripts/server-signing-key.sh."
fi

for file in "$@"; do
	[ -f "$file" ] || error "There's no $file to sign."
	openssl pkeyutl -sign -inkey "$work/key.pem" -rawin -in "$file" -out "$file.sig"
	openssl pkeyutl -verify -pubin -inkey "$work/public.pem" -rawin -in "$file" -sigfile "$file.sig" >/dev/null ||
		error "$file's new signature doesn't verify."
	(cd "$(dirname "$file")" && sha256sum "$(basename "$file")" >"$(basename "$file").sha256")
	echo "Signed $file"
done
