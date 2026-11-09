#!/bin/bash
# Puts dist/mac-<arch>/Nodus.app (packaging/macos/bundle.sh) in the DMG the
# release ships:
#
#   packaging/macos/dmg.sh <arm64|x64>
#
# Writes dist/Nodus-<arch>.dmg, made as electron-builder 26.15.3 made 0.9.4's
# (REL-010, REL-011, HANDOFF CONTRACT B): the same dmgbuild with the settings
# dmg-builder wrote. UDZO at zlib level 9 on HFS+, volume name
# "Nodus <version>-<arch>", no licence, no encryption, unsigned; Nodus.app, an
# Applications link, the volume icon (build/icon.icns) and dmg-builder's
# 540×380 background (background.tiff, MIT), laid out as Finder shows 0.9.4's.
#
# dmgbuild is electron-builder's self-contained bundle (dmgbuild 1.6.7 with its
# own Python), downloaded once into target/packaging and checked against the
# SHA-256 dmg-builder pins. DMGBUILD=<path to its dmgbuild script> uses
# another copy.
set -euo pipefail

arch="${1:-}"
case "$arch" in
  arm64 | x64) ;;
  *)
    echo "usage: $0 <arm64|x64>" >&2
    exit 2
    ;;
esac

root="$(cd "$(dirname "$0")/../.." && pwd)"
here="$root/packaging/macos"
app="$root/dist/mac-$arch/Nodus.app"
[ -d "$app" ] || {
  echo "No $app: run packaging/macos/bundle.sh $arch first" >&2
  exit 1
}
version="$(/usr/bin/plutil -extract CFBundleShortVersionString raw -o - "$app/Contents/Info.plist")"
volume="Nodus $version-$arch"
dmg="$root/dist/Nodus-$arch.dmg"

dmgbuild="${DMGBUILD:-}"
if [ -z "$dmgbuild" ]; then
  # dmg-builder 26.15.3's pins (node_modules/dmg-builder/out/dmgUtil.js).
  case "$(uname -m)" in
    arm64)
      bundle=dmgbuild-bundle-arm64-75c8a6c.tar.gz
      sha=793404d0c96687e27d5ee40a668d498c92e36a64d6c2906df511031adb33cbeb
      ;;
    *)
      bundle=dmgbuild-bundle-x86_64-75c8a6c.tar.gz
      sha=1664972f9cc2d6e8fce3b63e42cd30078aff602669c5856939c4519921200433
      ;;
  esac
  tools="$root/target/packaging/tools/${bundle%.tar.gz}"
  if [ ! -x "$tools/dmgbuild" ]; then
    rm -rf "$tools"
    mkdir -p "$tools"
    # A copy put there by hand (electron-builder's cache has one) is used as is.
    if [ ! -f "$tools.tar.gz" ]; then
      echo "▸ Downloading $bundle"
      curl -fsSL --retry 3 -o "$tools.tar.gz" \
        "https://github.com/electron-userland/electron-builder-binaries/releases/download/dmg-builder@1.2.5/$bundle"
    fi
    echo "$sha  $tools.tar.gz" | shasum -a 256 -c - >/dev/null || {
      echo "$bundle doesn't have the expected SHA-256" >&2
      rm -f "$tools.tar.gz"
      exit 1
    }
    tar -xzf "$tools.tar.gz" -C "$tools"
    rm -f "$tools.tar.gz"
  fi
  dmgbuild="$tools/dmgbuild"
fi

# The JSON dmg-builder's customizeDmg writes: no icon-size (dmgbuild's 80),
# text size 12, the window where a 380-high background puts it.
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
settings="$work/settings.json"
python3 - "$settings" "$volume" "$app" "$root/build/icon.icns" "$here/background.tiff" <<'PY'
import json, sys
out, title, app, icon, background = sys.argv[1:]
json.dump({
    "title": title,
    "text-size": 12,
    "compression-level": 9,
    "filesystem": "HFS+",
    "format": "UDZO",
    "contents": [
        {"path": app, "x": 130, "y": 220, "name": "Nodus.app", "type": "file"},
        {"path": "/Applications", "x": 410, "y": 220, "type": "link"},
    ],
    "icon": icon,
    "background": background,
    "window": {"position": {"x": 400, "y": 530}, "size": {"width": 540, "height": 380}},
}, open(out, "w"), indent=2)
PY

echo "▸ Making $dmg (volume \"$volume\")"
rm -f "$dmg"
PYTHONIOENCODING=utf8 "$dmgbuild" -s "$settings" "$volume" "$dmg"
echo "✓ $dmg"
