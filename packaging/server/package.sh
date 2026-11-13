#!/usr/bin/env bash
# Packs a built nodus-server as the release's
# nodus-server-linux-<x64|arm64>.tar.gz, with the binary named `nodus`, as
# install-server.sh unpacks it (SRV-201):
#
#   packaging/server/package.sh <x64|arm64> <path to nodus-server> [out folder, default dist]
#
# Refuses a binary for the other architecture, or one that needs a newer
# glibc than Ubuntu 22.04's 2.35 (Debian 12 has 2.36), since it wouldn't
# start on the oldest systems the install script accepts.
set -euo pipefail

GLIBC_FLOOR=2.35

[ $# -ge 2 ] || { echo "usage: package.sh <x64|arm64> <binary> [out folder]" >&2; exit 2; }
arch=$1 binary=$2 out=${3:-dist}
case "$arch" in
x64) machine="X86-64" ;;
arm64) machine="AArch64" ;;
*) echo "package.sh: the architecture is x64 or arm64, not $arch" >&2; exit 2 ;;
esac
[ -f "$binary" ] || { echo "package.sh: there's no $binary" >&2; exit 1; }

got=$(readelf -h "$binary" | awk -F: '/Machine:/ { gsub(/^ +/, "", $2); print $2 }')
case "$got" in
*"$machine"*) ;;
*) echo "package.sh: $binary is for $got, not $arch" >&2; exit 1 ;;
esac

needs=$(objdump -T "$binary" | grep -o 'GLIBC_[0-9][0-9.]*' | sed 's/GLIBC_//' | sort -uV | tail -n1)
if [ "$(printf '%s\n%s\n' "$needs" "$GLIBC_FLOOR" | sort -V | tail -n1)" != "$GLIBC_FLOOR" ]; then
	echo "package.sh: $binary needs glibc $needs, newer than $GLIBC_FLOOR (Ubuntu 22.04): build it on Ubuntu 22.04" >&2
	exit 1
fi

stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
install -m 0755 "$binary" "$stage/nodus"
mkdir -p "$out"
asset="$out/nodus-server-linux-$arch.tar.gz"
tar --owner=0 --group=0 --numeric-owner -C "$stage" -czf "$asset" nodus
echo "$asset: nodus for $got, needs glibc $needs, $(wc -c <"$asset" | tr -d ' ') bytes"
