#!/bin/bash
# The macOS handoff gate (decision D12): replays what every installed Electron
# copy (0.6.0 … 0.9.4, src/main/updater.ts, unchanged since b31d09b) does with
# a downloaded DMG before it swaps the app in, with the same commands and
# arguments, and fails where 0.9.4 would say the update can't be installed.
#
#   packaging/macos/handoff-gate.sh <dmg> <arm64|x64> <version> [<running Nodus.app>]
#
# <version> is the release's (the tag without its v). <running Nodus.app> is
# the copy doing the updating, e.g. a real 0.9.4 copied out of its own DMG:
# its bundle id is compared, and when it has a team, the new app must have
# the same one (UPD-046). Without it, the bundle id is dev.saleem.nodus and
# there's no team to match, as for the ad-hoc 0.6.0–0.8.0 copies.
#
#   GATE_TEAM_ID=<id>      the new app's team must be this one (never printed)
#   GATE_NOTARIZED=1       the new app must also pass Gatekeeper as notarized,
#                          with its ticket stapled (manual downloads)
#   GATE_COPY_TO=<dir>     keeps the copy 0.9.4 would swap in at <dir>/Nodus.app,
#                          for the smoke test to launch
#
# Beyond 0.9.4's own checks it holds the rest of the HANDOFF CONTRACT that
# nothing checks at update time: a thin binary of the DMG's arch, macOS 13.0,
# exactly one app named Nodus.app, the DMG's format, volume name and layout.
set -euo pipefail

dmg="${1:-}" arch="${2:-}" version="${3:-}" running="${4:-}"
case "$arch" in
  arm64) lipo_arch=arm64 ;;
  x64) lipo_arch=x86_64 ;;
  *)
    echo "usage: $0 <dmg> <arm64|x64> <version> [<running Nodus.app>]" >&2
    exit 2
    ;;
esac
[ -f "$dmg" ] && [ -n "$version" ] || {
  echo "usage: $0 <dmg> <arm64|x64> <version> [<running Nodus.app>]" >&2
  exit 2
}
root="$(cd "$(dirname "$0")/../.." && pwd)"

fail() {
  echo "✗ $*" >&2
  exit 1
}
pass() { echo "  ✓ $*"; }

# A throwaway <userData>/updates.
work="$(mktemp -d)"
updates="$work/updates"
mkdir -p "$updates"
mount=""
cleanup() {
  if [ -n "$mount" ] && [ -d "$mount" ]; then
    /usr/bin/hdiutil detach "$mount" -force >/dev/null 2>&1 || true
  fi
  rm -rf "$work"
}
trap cleanup EXIT

echo "▸ $dmg as 0.9.4's updater sees it"

# The DMG itself (REL-010): UDZO, unencrypted, no licence to agree to.
info="$work/imageinfo.plist"
/usr/bin/hdiutil imageinfo -plist "$dmg" >"$info" 2>/dev/null
[ "$(/usr/bin/plutil -extract Format raw -o - "$info")" = "UDZO" ] || fail "The DMG isn't UDZO"
[ "$(/usr/bin/plutil -extract Properties.Encrypted raw -o - "$info")" = "false" ] || fail "The DMG is encrypted"
[ "$(/usr/bin/plutil -extract 'Properties.Software License Agreement' raw -o - "$info")" = "false" ] ||
  fail "The DMG has a licence agreement"
pass "UDZO, unencrypted, no licence agreement"

# copyApp: mkdtemp(updates/mount-), attach hidden and read-only.
mount="$(mktemp -d "$updates/mount-XXXXXX")"
/usr/bin/hdiutil attach "$dmg" -nobrowse -noautoopen -readonly -mountpoint "$mount" >/dev/null ||
  fail "hdiutil couldn't attach it (0.9.4: The update couldn’t be prepared. Try again later.)"
pass "attached with -nobrowse -noautoopen -readonly -mountpoint"

volume="$(/usr/sbin/diskutil info -plist "$mount" | /usr/bin/plutil -extract VolumeName raw -o - -)"
[ "$volume" = "Nodus $version-$arch" ] || fail "The volume is '$volume', not 'Nodus $version-$arch'"
filesystem="$(/usr/sbin/diskutil info -plist "$mount" | /usr/bin/plutil -extract FilesystemType raw -o - -)"
[ "$filesystem" = "hfs" ] || fail "The volume is $filesystem, not HFS+"
pass "volume \"$volume\", HFS+"

# readdir(mount).find((n) => n.endsWith('.app')): the first in directory order.
name="$(ls -f "$mount" | grep '\.app$' | head -1 || true)"
[ -n "$name" ] || fail "No app at the volume root (0.9.4: The update has no app in it.)"
apps="$(ls -f "$mount" | grep -c '\.app$' || true)"
[ "$apps" = 1 ] && [ "$name" = "Nodus.app" ] || fail "The volume root should hold Nodus.app alone, not: $(ls -f "$mount" | grep '\.app$' | tr '\n' ' ')"
[ "$(readlink "$mount/Applications")" = "/Applications" ] || fail "No Applications link"
cmp -s "$mount/.VolumeIcon.icns" "$root/build/icon.icns" || fail "The volume icon isn't build/icon.icns"
[ -f "$mount/.background.tiff" ] && [ -f "$mount/.DS_Store" ] || fail "No background or Finder layout"
pass "Nodus.app, Applications -> /Applications, volume icon, background, layout"

# --noqtn: the copy carries no quarantine flag.
target="$updates/$version/Nodus.app"
mkdir -p "$updates/$version"
/usr/bin/ditto --noqtn "$mount/$name" "$target" || fail "ditto couldn't copy it"
pass "copied with ditto --noqtn"
/usr/bin/hdiutil detach "$mount" -force >/dev/null 2>&1 || echo "  (detach failed; 0.9.4 only logs that)"
rmdir "$mount" 2>/dev/null || true
mount=""

# verify(): deep strict signature, then the plist reads, then the team.
/usr/bin/codesign --verify --deep --strict "$target" 2>/dev/null ||
  fail "codesign --verify --deep --strict failed (0.9.4: The new version’s signature didn’t check out.)"
pass "codesign --verify --deep --strict"

plist_value() { /usr/bin/plutil -extract "$2" raw -o - "$1/Contents/Info.plist"; }
id="$(plist_value "$target" CFBundleIdentifier)" || fail "No CFBundleIdentifier (0.9.4: The update couldn’t be prepared. Try again later.)"
new_version="$(plist_value "$target" CFBundleShortVersionString)" || fail "No CFBundleShortVersionString (0.9.4: The update couldn’t be prepared. Try again later.)"
if [ -n "$running" ]; then
  own_id="$(plist_value "$running" CFBundleIdentifier)"
else
  own_id="dev.saleem.nodus"
fi
[ "$id" = "$own_id" ] && [ "$new_version" = "$version" ] ||
  fail "Bundle id '$id' / version '$new_version', expected '$own_id' / '$version' (0.9.4: The update isn’t the expected version of Nodus.)"
pass "CFBundleIdentifier $id, CFBundleShortVersionString $new_version"

# teamId(): TeamIdentifier on codesign -dv's stderr; 'not set' is none.
team_of() {
  local team
  team="$(/usr/bin/codesign -dv "$1" 2>&1 >/dev/null | sed -n 's/^TeamIdentifier=//p' | head -1)"
  [ "$team" = "not set" ] && team=""
  printf '%s' "$team"
}
new_team="$(team_of "$target")"
if [ -n "$running" ]; then
  own_team="$(team_of "$running")"
  if [ -n "$own_team" ]; then
    [ "$new_team" = "$own_team" ] ||
      fail "Not signed by the running copy's team (0.9.4: The update isn’t signed by the same developer.)"
    pass "same team as the running copy"
  else
    pass "the running copy is ad-hoc signed, so 0.9.4 skips the team"
  fi
fi
if [ -n "${GATE_TEAM_ID:-}" ]; then
  [ "$new_team" = "$GATE_TEAM_ID" ] || fail "Not signed by the expected team"
  pass "signed by the expected team"
fi
[ -n "$new_team" ] || echo "  (ad-hoc signed: every 0.8.1+ copy would refuse it)"

# The rest of the contract, which nothing checks before the swap.
exe="$(plist_value "$target" CFBundleExecutable)"
[ "$exe" = "Nodus" ] && [ -x "$target/Contents/MacOS/Nodus" ] || fail "The executable isn't Contents/MacOS/Nodus"
[ "$(/usr/bin/lipo -archs "$target/Contents/MacOS/Nodus")" = "$lipo_arch" ] || fail "Nodus isn't a thin $lipo_arch binary"
minos="$(/usr/bin/vtool -show-build "$target/Contents/MacOS/Nodus" | awk '$1 == "minos" { print $2; exit }')"
[ "$minos" = "13.0" ] && [ "$(plist_value "$target" LSMinimumSystemVersion)" = "13.0" ] || fail "Not built for macOS 13.0"
# Both sides resolved, since the temp folder is behind /var -> /private/var.
inside="$(cd "$target" && pwd -P)"
while IFS= read -r link; do
  points="$(cd "$(dirname "$link")" && realpath "$(readlink "$link")" 2>/dev/null || echo /)"
  case "$points" in
    "$inside"/*) ;;
    *) fail "A link points out of the bundle: $link" ;;
  esac
done < <(find "$target" -type l)
flags="$(/usr/bin/codesign -dv "$target" 2>&1 >/dev/null | sed -n 's/^CodeDirectory .*flags=\(0x[0-9a-f]*\)(\([^)]*\)).*/\2/p')"
case ",$flags," in
  *,runtime,*) ;;
  *) fail "No hardened runtime (flags: $flags)" ;;
esac
entitlements="$(/usr/bin/codesign -d --entitlements - --xml "$target" 2>/dev/null)"
for key in com.apple.security.device.audio-input com.apple.security.automation.apple-events; do
  grep -q "<key>$key</key>" <<<"$entitlements" || fail "No $key entitlement"
done
pass "Contents/MacOS/Nodus, thin $lipo_arch, macOS 13.0, hardened runtime, mic and Apple Events entitlements"

if [ "${GATE_NOTARIZED:-}" = 1 ]; then
  /usr/bin/xcrun stapler validate "$target" >/dev/null || fail "No notarization ticket stapled"
  /usr/sbin/spctl --assess --type execute "$target" 2>/dev/null || fail "Gatekeeper rejects it"
  pass "notarized, stapled, accepted by Gatekeeper"
fi

if [ -n "${GATE_COPY_TO:-}" ]; then
  mkdir -p "$GATE_COPY_TO"
  rm -rf "$GATE_COPY_TO/Nodus.app"
  /usr/bin/ditto --noqtn "$target" "$GATE_COPY_TO/Nodus.app"
fi
echo "✓ 0.9.4 would install $dmg"
