#!/bin/bash
# Builds Nodus.app for one architecture, the way the release ships it:
#
#   packaging/macos/bundle.sh <arm64|x64> [--no-build]
#
# Writes dist/mac-<arch>/Nodus.app. The binary is built with NODUS_PACKAGED=1
# (a packaged build: it updates itself, nodus_platform::app::is_packaged) and a
# macOS 13.0 deployment target, thin for the one architecture. It is both the
# app and the session host (`Nodus host …`, decision D5), so the bundle holds a
# single Mach-O, signed once, with the hardened runtime and
# packaging/macos/entitlements.plist.
#
# Signing:
#   NODUS_SIGN_IDENTITY  a codesign identity, e.g. "Developer ID Application"
#                        (found in the keychain search list, where the release
#                        workflow puts the CSC_LINK certificate). Unset: ad-hoc
#                        signed, which every macOS copy runs but 0.9.4's
#                        updater rejects (it wants the same team).
# Notarizing, only with an identity and when all three are set (as
# electron-builder did): APPLE_ID, APPLE_APP_SPECIFIC_PASSWORD, APPLE_TEAM_ID.
# Their values are never printed.
#
# --no-build packages the binary already in target/<triple>/release.
set -euo pipefail

arch="${1:-}"
case "$arch" in
  arm64) triple=aarch64-apple-darwin lipo_arch=arm64 ;;
  x64) triple=x86_64-apple-darwin lipo_arch=x86_64 ;;
  *)
    echo "usage: $0 <arm64|x64> [--no-build]" >&2
    exit 2
    ;;
esac
build=1
[ "${2:-}" = "--no-build" ] && build=0

root="$(cd "$(dirname "$0")/../.." && pwd)"
here="$root/packaging/macos"
cd "$root"

# One version for everything (REL-001): the workspace's.
version="$(cargo pkgid -p nodus | sed 's/.*[#@]//')"
year="$(date +%Y)"
target_dir="${CARGO_TARGET_DIR:-$root/target}"
binary="$target_dir/$triple/release/nodus"

if [ "$build" = 1 ]; then
  echo "▸ Building nodus $version for $triple (packaged, macOS 13.0)"
  NODUS_PACKAGED=1 MACOSX_DEPLOYMENT_TARGET=13.0 \
    cargo build --release --locked --target "$triple" -p nodus
fi
[ -x "$binary" ] || {
  echo "No binary at $binary" >&2
  exit 1
}

out="$root/dist/mac-$arch"
app="$out/Nodus.app"
echo "▸ Assembling $app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$binary" "$app/Contents/MacOS/Nodus"
sed -e "s/@VERSION@/$version/g" -e "s/@YEAR@/$year/g" "$here/Info.plist" >"$app/Contents/Info.plist"
/usr/bin/plutil -lint "$app/Contents/Info.plist" >/dev/null
printf 'APPL????' >"$app/Contents/PkgInfo"
cp "$root/build/icon.icns" "$app/Contents/Resources/icon.icns"

# Thin, and launchable on macOS 13 (HANDOFF CONTRACT 18): nothing checks this
# before 0.9.4 swaps the app in.
archs="$(/usr/bin/lipo -archs "$app/Contents/MacOS/Nodus")"
[ "$archs" = "$lipo_arch" ] || {
  echo "Nodus is built for '$archs', not $lipo_arch" >&2
  exit 1
}
minos="$(/usr/bin/vtool -show-build "$app/Contents/MacOS/Nodus" | awk '$1 == "minos" { print $2; exit }')"
[ "$minos" = "13.0" ] || {
  echo "Nodus needs macOS $minos, not 13.0" >&2
  exit 1
}

identity="${NODUS_SIGN_IDENTITY:-}"
if [ -n "$identity" ]; then
  echo "▸ Signing with the $identity identity (hardened runtime, timestamped)"
  /usr/bin/codesign --force --timestamp --options runtime \
    --entitlements "$here/entitlements.plist" --sign "$identity" "$app"
else
  echo "▸ Signing ad-hoc (hardened runtime); set NODUS_SIGN_IDENTITY for a Developer ID"
  /usr/bin/codesign --force --options runtime \
    --entitlements "$here/entitlements.plist" --sign - "$app"
fi
/usr/bin/codesign --verify --deep --strict "$app"
# The signing identifier is the bundle id, so the designated requirement is
# 0.9.4's and TCC grants and the Keychain item carry over (REL-046).
identifier="$(/usr/bin/codesign -dv "$app" 2>&1 | sed -n 's/^Identifier=//p')"
[ "$identifier" = "dev.saleem.nodus" ] || {
  echo "Signed as '$identifier', not dev.saleem.nodus" >&2
  exit 1
}

if [ -n "$identity" ] && [ -n "${APPLE_ID:-}" ] && [ -n "${APPLE_APP_SPECIFIC_PASSWORD:-}" ] && [ -n "${APPLE_TEAM_ID:-}" ]; then
  echo "▸ Notarizing"
  work="$(mktemp -d)"
  trap 'rm -rf "$work"' EXIT
  /usr/bin/ditto -c -k --keepParent "$app" "$work/Nodus.zip"
  /usr/bin/xcrun notarytool submit "$work/Nodus.zip" \
    --apple-id "$APPLE_ID" --password "$APPLE_APP_SPECIFIC_PASSWORD" --team-id "$APPLE_TEAM_ID" \
    --wait --output-format json >"$work/result.json"
  status="$(/usr/bin/plutil -extract status raw -o - "$work/result.json")"
  if [ "$status" != "Accepted" ]; then
    echo "Notarization answered '$status'" >&2
    submission="$(/usr/bin/plutil -extract id raw -o - "$work/result.json" 2>/dev/null || true)"
    if [ -n "$submission" ]; then
      /usr/bin/xcrun notarytool log "$submission" \
        --apple-id "$APPLE_ID" --password "$APPLE_APP_SPECIFIC_PASSWORD" --team-id "$APPLE_TEAM_ID" >&2 || true
    fi
    exit 1
  fi
  # Stapling adds the ticket outside the sealed resources, so the signature
  # still verifies.
  /usr/bin/xcrun stapler staple "$app"
  /usr/bin/xcrun stapler validate "$app"
  /usr/bin/codesign --verify --deep --strict "$app"
elif [ -n "$identity" ]; then
  echo "▸ Not notarizing: APPLE_ID, APPLE_APP_SPECIFIC_PASSWORD and APPLE_TEAM_ID aren't all set"
fi

echo "✓ $app ($version, $lipo_arch)"
