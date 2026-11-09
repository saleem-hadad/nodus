#!/bin/bash
# Builds the Windows installer the release ships, for one architecture:
#
#   packaging/windows/installer.sh <x64|arm64> [--no-build]
#
# Writes dist/Nodus-Setup-<arch>.exe: electron-builder 26.15.3's one-click,
# per-user NSIS installer (decision D9), from its own templates
# (packaging/windows/nsis, unchanged), its own makensis (3.0.4.1) and plugins
# (nsis-resources 3.4.1), driven with the defines and commands its NsisTarget
# passed for 0.9.4. So the installer that 0.9.4 runs as
# `Nodus-Setup-<arch>.exe --updated /S --force-run` finds the install by the
# same registry id, installs into the same folder, runs the old uninstaller
# keeping data and shortcuts, writes the same registry values and starts the
# app (HANDOFF CONTRACT E, REL-016…REL-026).
#
# Like electron-builder: the app folder (dist/win-<arch>-unpacked, Nodus.exe
# alone) goes in as a 7z archive the installer extracts, and the uninstaller
# is made first, by compiling the script with BUILD_UNINSTALLER and running
# that installer, which only writes `Uninstall Nodus.exe` and quits.
#
# Runs on Windows (Git Bash, as the workflows' default shell). On a Mac or
# Linux, --compile-only checks that both passes compile, with a stand-in
# uninstaller, since the first pass's installer can't run there.
#
# Tools, each downloaded once into target/packaging and checked against the
# SHA-256 electron-builder pins unless named:
#   NSIS_DIR             electron-builder's nsis-3.0.4.1 (Bin/makensis.exe, mac/makensis)
#   NSIS_RESOURCES_DIR   electron-builder's nsis-resources-3.4.1 (plugins/x86-unicode)
#   SEVEN_ZIP            7z, 7zz or 7za (Windows runners have 7z)
#
# --no-build packages the binary already in target/<triple>/release.
set -euo pipefail

arch="" build=1 compile_only=0
for arg in "$@"; do
  case "$arg" in
    x64 | arm64) arch="$arg" ;;
    --no-build) build=0 ;;
    --compile-only) compile_only=1 build=0 ;;
    *)
      echo "usage: $0 <x64|arm64> [--no-build] [--compile-only]" >&2
      exit 2
      ;;
  esac
done
case "$arch" in
  x64) triple=x86_64-pc-windows-msvc app_define=APP_64 ;;
  arm64) triple=aarch64-pc-windows-msvc app_define=APP_ARM64 ;;
  *)
    echo "usage: $0 <x64|arm64> [--no-build] [--compile-only]" >&2
    exit 2
    ;;
esac

windows=0
case "$(uname -s)" in MINGW* | MSYS* | CYGWIN*) windows=1 ;; esac
[ "$windows" = 1 ] || [ "$compile_only" = 1 ] || {
  echo "The installer is made on Windows; elsewhere only --compile-only works" >&2
  exit 2
}
# Paths as makensis and 7z, which are Windows programs there, read them.
native() { if [ "$windows" = 1 ]; then cygpath -w "$1"; else printf '%s' "$1"; fi; }

root="$(cd "$(dirname "$0")/../.." && pwd)"
here="$root/packaging/windows"
cd "$root"
tools="$root/target/packaging/tools"
mkdir -p "$tools" "$root/dist"

seven="${SEVEN_ZIP:-}"
if [ -z "$seven" ]; then
  for candidate in 7z 7zz 7za; do
    command -v "$candidate" >/dev/null && seven="$candidate" && break
  done
fi

sha256() { if command -v sha256sum >/dev/null; then sha256sum "$1"; else shasum -a 256 "$1"; fi | cut -d' ' -f1; }
sha512() { if command -v sha512sum >/dev/null; then sha512sum "$1"; else shasum -a 512 "$1"; fi | cut -d' ' -f1; }

# fetch <name> <sha256>: electron-builder-binaries' <name>/<name>.7z, extracted once.
fetch() {
  local name="$1" sha="$2" dir="$tools/$1"
  if [ ! -d "$dir" ]; then
    [ -n "$seven" ] || {
      echo "No 7z to extract $name with: set SEVEN_ZIP, or ${3} to an extracted copy" >&2
      exit 1
    }
    echo "▸ Downloading $name" >&2
    curl -fsSL --retry 3 -o "$dir.7z" \
      "https://github.com/electron-userland/electron-builder-binaries/releases/download/$name/$name.7z"
    [ "$(sha256 "$dir.7z")" = "$sha" ] || {
      echo "$name.7z doesn't have the expected SHA-256" >&2
      rm -f "$dir.7z"
      exit 1
    }
    "$seven" x -bd -y "-o$(native "$dir.tmp")" "$(native "$dir.7z")" >/dev/null
    mv "$dir.tmp" "$dir"
    rm -f "$dir.7z"
  fi
  printf '%s' "$dir"
}
# app-builder-lib 26.15.3's pins (out/toolsets/windows.js): the "legacy" NSIS
# it uses when no toolset is configured, as for 0.9.4.
nsis="${NSIS_DIR:-$(fetch nsis-3.0.4.1 9877df902530f96357d13a7a31ae2b9df67f48b11ffc9a1700a7c961574ec5fa NSIS_DIR)}"
resources="${NSIS_RESOURCES_DIR:-$(fetch nsis-resources-3.4.1 593a9a92ef958321293ac6a2ee61e64bf1bd543142a5bd6b3d310709cc924103 NSIS_RESOURCES_DIR)}"
if [ "$windows" = 1 ]; then
  makensis="$nsis/Bin/makensis.exe"
elif [ "$(uname -s)" = Darwin ]; then
  makensis="$nsis/mac/makensis"
else
  makensis="$nsis/linux/makensis"
fi
[ -x "$makensis" ] || {
  echo "No makensis at $makensis" >&2
  exit 1
}

# One version for everything (REL-001): the workspace's.
version="$(cargo pkgid -p nodus | sed 's/.*[#@]//')"
year="$(date +%Y)"
target_dir="${CARGO_TARGET_DIR:-$root/target}"

app="$root/dist/win-$arch-unpacked"
if [ "$compile_only" = 1 ]; then
  # Any file will do as the app for checking the script.
  rm -rf "$app"
  mkdir -p "$app"
  printf 'stand-in for Nodus.exe' >"$app/Nodus.exe"
else
  if [ "$build" = 1 ]; then
    echo "▸ Building nodus $version for $triple (packaged)"
    NODUS_PACKAGED=1 cargo build --release --locked --target "$triple" -p nodus
  fi
  binary="$target_dir/$triple/release/nodus.exe"
  [ -f "$binary" ] || {
    echo "No binary at $binary" >&2
    exit 1
  }
  rm -rf "$app"
  mkdir -p "$app"
  cp "$binary" "$app/Nodus.exe"
fi

# The app package: electron-builder's archive("7z", …, { withoutDir: true }).
package_name="nodus-$version-$arch.nsis.7z"
package="$root/dist/$package_name"
rm -f "$package"
if [ "$compile_only" = 1 ] && [ -z "$seven" ]; then
  printf 'stand-in archive' >"$package"
else
  (cd "$app" && "$seven" a -bd -mx=9 -mtc=off -mtm=off -mta=off "$(native "$package")" . \
    '-xr!*.avi' '-xr!*.mov' '-xr!*.m4v' '-xr!*.mp4' '-xr!*.m4p' '-xr!*.qt' '-xr!*.mkv' '-xr!*.webm' '-xr!*.vmdk' >/dev/null)
fi
unpacked="$(find "$app" -type f -print0 | xargs -0 cat | wc -c | tr -d ' ')"

installer="$root/dist/Nodus-Setup-$arch.exe"
uninstaller="$root/dist/Nodus-Setup-$arch.__uninstaller.exe"
icon="$here/icon.ico"
description="A spatial board for managing Claude Code sessions per project"

# NsisTarget.buildInstaller's defines, in its order. APP_GUID is UUIDv5 of
# the app id in electron-builder's namespace, and must never change.
defines=(
  "APP_ID=dev.saleem.nodus"
  "APP_GUID=99c2a492-c8ad-53bf-923b-1df868e91b51"
  "UNINSTALL_APP_KEY=99c2a492-c8ad-53bf-923b-1df868e91b51" # gitleaks:allow (a public UUIDv5, not a secret)
  "PRODUCT_NAME=Nodus"
  "PRODUCT_FILENAME=Nodus"
  "APP_FILENAME=nodus"
  "APP_DESCRIPTION=$description"
  "VERSION=$version"
  "PROJECT_DIR=$(native "$root")"
  "BUILD_RESOURCES_DIR=$(native "$root/build")"
  "APP_PACKAGE_NAME=nodus"
  "MUI_ICON=$(native "$icon")"
  "MUI_UNICON=$(native "$icon")"
  "$app_define=$(native "$package")"
  "${app_define}_NAME=$package_name"
  "${app_define}_HASH=$(sha512 "$package" | tr '[:lower:]' '[:upper:]')"
  "${app_define}_UNPACKED_SIZE=$(((unpacked + 1023) / 1024))"
  "COMPANY_NAME=Saleem Hadad"
  "APP_PRODUCT_FILENAME=Nodus"
  "APP_INSTALLER_STORE_FILE=nodus-updater\\installer.exe"
  "COMPRESSION_METHOD=7z"
  "ONE_CLICK"
  "RUN_AFTER_FINISH"
  "SHORTCUT_NAME=Nodus"
  "UNINSTALL_DISPLAY_NAME=Nodus $version"
  "ESTIMATED_SIZE=$(((unpacked + 512) / 1024))"
  "COMPRESS=auto"
)
# computeVersionKey: the installer's version resource.
commands=(
  "-XVIProductVersion $version.0"
  "-XVIAddVersionKey /LANG=1033 ProductName \"Nodus\""
  "-XVIAddVersionKey /LANG=1033 ProductVersion \"$version\""
  "-XVIAddVersionKey /LANG=1033 LegalCopyright \"Copyright © $year Saleem Hadad\""
  "-XVIAddVersionKey /LANG=1033 FileDescription \"$description\""
  "-XVIAddVersionKey /LANG=1033 FileVersion \"$version\""
  "-XVIAddVersionKey /LANG=1033 CompanyName \"Saleem Hadad\""
  "-XUnicode true"
  "-XSetCompressor zlib"
)

# The script: electron-builder's header, then installer.nsi.
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
sed -e "s|@TEMPLATES@|$(native "$here/nsis")|g" \
  -e "s|@PLUGINS@|$(native "$resources/plugins")|g" \
  -e "s|@GENERATED@|$(native "$here/nsis-header")|g" \
  "$here/nsis-header/header.nsh" >"$work/script.nsi"
cat "$here/nsis/installer.nsi" >>"$work/script.nsi"

# makensis <extra define>…: one pass, reading the script on stdin from the
# templates folder, as executeMakensis did (warnings are errors).
makensis() {
  local args=(-WX -INPUTCHARSET UTF8)
  local define
  for define in "${defines[@]}" "$@"; do args+=("-D$define"); done
  args+=("-XOutFile \"$(native "$installer")\"" "${commands[@]}" -)
  (cd "$here/nsis" && MSYS_NO_PATHCONV=1 MSYS2_ARG_CONV_EXCL='*' NSISDIR="$(native "$nsis")" \
    "$makensis" "${args[@]}" <"$work/script.nsi" >"$work/makensis.log" 2>&1) || {
    cat "$work/makensis.log" >&2
    exit 1
  }
  grep -E '^(Output|Install|Uninstall|Total size):' "$work/makensis.log" | sed 's/^/  /' || true
}

echo "▸ Making the uninstaller"
rm -f "$installer" "$uninstaller"
makensis BUILD_UNINSTALLER "UNINSTALLER_OUT_FILE=$(native "$uninstaller")"
if [ "$compile_only" = 1 ]; then
  cp "$installer" "$uninstaller"
else
  # The first pass's installer writes the uninstaller and quits.
  __COMPAT_LAYER=RunAsInvoker "$installer"
  [ -f "$uninstaller" ] || {
    echo "The uninstaller wasn't written" >&2
    exit 1
  }
fi

echo "▸ Making $installer ($version, $arch)"
makensis "UNINSTALLER_OUT_FILE=$(native "$uninstaller")"
rm -f "$uninstaller" "$package"
if [ "$compile_only" = 1 ]; then
  rm -f "$installer"
  echo "✓ Both passes compile (--compile-only: no installer kept)"
else
  echo "✓ $installer"
fi
