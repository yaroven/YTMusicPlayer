#!/usr/bin/env bash
# Builds the macOS installer package from one or more `ytm` binaries
# (arm64 and/or x86_64; several are merged into a universal binary):
#
#   packaging/macos/build-pkg.sh VERSION OUT.pkg BINARY...
#
# The package installs /Applications/ytm-player.app and a `ytm` command
# (/usr/local/bin/ytm -> the app's binary). Unsigned: on first open, macOS
# asks to confirm in System Settings > Privacy & Security ("Open Anyway").
set -euo pipefail

version=$1 out=$2
shift 2
[[ $# -ge 1 ]] || {
  echo "usage: $0 VERSION OUT.pkg BINARY..." >&2
  exit 2
}
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

app="$work/root/Applications/ytm-player.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
if [[ $# -gt 1 ]]; then
  lipo -create "$@" -output "$app/Contents/MacOS/ytm-player"
else
  cp "$1" "$app/Contents/MacOS/ytm-player"
fi
chmod 755 "$app/Contents/MacOS/ytm-player"
cp "$repo/assets/ytm-player.icns" "$app/Contents/Resources/"
sed "s/@VERSION@/$version/g" "$here/Info.plist" >"$app/Contents/Info.plist"
# No extended attributes (they'd end up as ._ files in the payload).
xattr -cr "$work/root"
export COPYFILE_DISABLE=1
# Ad-hoc signature: required to run on Apple silicon, and a stable identity
# for the Keychain entry that holds the sign-in.
codesign --force --sign - "$app"

# Don't let Installer "relocate" the app into an older copy elsewhere with
# the same bundle id (e.g. ~/Applications from install.sh).
pkgbuild --analyze --root "$work/root" "$work/components.plist" >/dev/null
plutil -replace 0.BundleIsRelocatable -bool NO "$work/components.plist"

pkgbuild --root "$work/root" \
  --component-plist "$work/components.plist" \
  --scripts "$here/scripts" \
  --identifier dev.ytm-player.pkg \
  --version "$version" \
  --install-location / \
  "$work/ytm-player.pkg" >/dev/null
productbuild --package "$work/ytm-player.pkg" "$out" >/dev/null
echo "$out"
