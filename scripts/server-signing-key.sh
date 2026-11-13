#!/usr/bin/env bash
# Makes the ed25519 key that releases sign the Linux server and
# install-server.sh with (SRV-201). Run it once, on your own machine, from
# the repo, with gh signed in as someone who can set the repo's secrets:
#
#   scripts/server-signing-key.sh [--backup <file outside the repo>] [--rotate]
#
# It:
#   1. generates the keypair in a private temporary folder;
#   2. sets the repo secret NODUS_SERVER_SIGNING_KEY to the private key
#      (gh secret set, from standard input, so it's never on a command line);
#   3. writes the public key into packaging/server/install-server.sh,
#      between its SIGNING KEY markers, and prints it, for the script and
#      for the server binary, which builds it in for updates (SRV-200);
#   4. deletes the private key, unless --backup names a file to keep a copy
#      in (a password manager is a better home for it than a disk).
#
# The private key never goes in the repo or on screen. Commit the changed
# install-server.sh afterwards: the release refuses to sign when the key in
# the script isn't the secret's.
#
# Every installed server trusts this key, so replacing it breaks their
# updates until they run a new install-server.sh. That's why it refuses
# when the secret is set already, unless --rotate is given.
set -euo pipefail

REPO="saleem-hadad/nodus"
SECRET="NODUS_SERVER_SIGNING_KEY"
backup="" rotate=0
while [ $# -gt 0 ]; do
	case "$1" in
	--backup)
		[ $# -ge 2 ] || { echo "server-signing-key.sh: --backup needs a file" >&2; exit 2; }
		backup=$2
		shift
		;;
	--rotate) rotate=1 ;;
	-h | --help)
		sed -n '2,/^$/s/^# \{0,1\}//p' "$0"
		exit 0
		;;
	*) echo "server-signing-key.sh: unknown option $1" >&2; exit 2 ;;
	esac
	shift
done

die() { echo "server-signing-key.sh: $*" >&2; exit 1; }

root=$(git rev-parse --show-toplevel 2>/dev/null) || die "run it from inside the nodus repo"
script="$root/packaging/server/install-server.sh"
[ -f "$script" ] || die "there's no $script"
if ! grep -q '^# BEGIN SIGNING KEY$' "$script" || ! grep -q '^# END SIGNING KEY$' "$script"; then
	die "$script has no SIGNING KEY markers"
fi
command -v openssl >/dev/null || die "openssl is missing"
openssl version | grep -qE '^OpenSSL [3-9]' || die "this needs OpenSSL 3 or later for ed25519 (have: $(openssl version))"
command -v gh >/dev/null || die "gh is missing"
gh auth status >/dev/null 2>&1 || die "gh isn't signed in: gh auth login"

if gh secret list --repo "$REPO" 2>/dev/null | awk '{print $1}' | grep -qx "$SECRET"; then
	[ "$rotate" = 1 ] || die "$SECRET is set already. Replacing it breaks updates on every installed server; pass --rotate if that's what you want."
fi

if [ -n "$backup" ]; then
	[ ! -e "$backup" ] || die "$backup exists already"
	backup_dir=$(cd "$(dirname "$backup")" && pwd -P) || die "no folder for $backup"
	case "$backup_dir/" in
	"$(cd "$root" && pwd -P)"/*) die "keep the backup outside the repo, not in $backup_dir" ;;
	esac
fi

umask 077
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
openssl genpkey -algorithm ed25519 -out "$work/private.pem"
openssl pkey -in "$work/private.pem" -pubout -out "$work/public.pem"

# A round trip before anything is stored.
printf 'nodus' >"$work/probe"
openssl pkeyutl -sign -inkey "$work/private.pem" -rawin -in "$work/probe" -out "$work/probe.sig"
openssl pkeyutl -verify -pubin -inkey "$work/public.pem" -rawin -in "$work/probe" -sigfile "$work/probe.sig" >/dev/null ||
	die "the new key doesn't verify its own signature"

gh secret set "$SECRET" --repo "$REPO" <"$work/private.pem"
echo "Set the secret $SECRET on $REPO."

if [ -n "$backup" ]; then
	cp "$work/private.pem" "$backup"
	chmod 600 "$backup"
	echo "Kept a copy of the private key in $backup."
fi

# The block between the markers, replaced whole.
awk -v keyfile="$work/public.pem" '
	/^# BEGIN SIGNING KEY$/ {
		print
		key = ""
		while ((getline line < keyfile) > 0) key = key (key == "" ? "" : "\n") line
		print "readonly SIGNING_KEY='\''" key "'\''"
		skip = 1
		next
	}
	/^# END SIGNING KEY$/ { skip = 0 }
	!skip { print }
' "$script" >"$work/install-server.sh"
cat "$work/install-server.sh" >"$script"
echo "Wrote the public key into ${script#"$root"/}: commit it."

echo
echo "The public key, for install-server.sh and the server binary (SRV-200):"
echo
cat "$work/public.pem"
echo
echo "SHA-256 of its DER form: $(openssl pkey -pubin -in "$work/public.pem" -outform DER | openssl dgst -sha256 -r | cut -d' ' -f1)"
