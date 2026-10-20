#!/bin/bash
# Regenerates the app icon on every platform from draw-icon.swift (macOS:
# needs swiftc and iconutil). Usage: packaging/icon/make-icons.sh
set -euo pipefail
repo="$(cd "$(dirname "$0")/../.." && pwd)"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
swiftc -O -o "$work/draw-icon" "$repo/packaging/icon/draw-icon.swift"
"$work/draw-icon" "$work"

# macOS, Linux (256), window and tray (64)
iconutil -c icns -o "$repo/assets/yarek-music.icns" "$work/yarek-music.iconset"
cp "$work/rounded-256.png" "$repo/assets/yarek-music.png"
cp "$work/rounded-64.png" "$repo/assets/yarek-music-64.png"

# Windows: an .ico of PNG frames
python3 - "$repo/assets/yarek-music.ico" "$work" <<'PY'
import struct, sys
out, work = sys.argv[1], sys.argv[2]
sizes = [16, 24, 32, 48, 64, 128, 256]
frames = [open(f"{work}/rounded-{s}.png", "rb").read() for s in sizes]
header = struct.pack("<HHH", 0, 1, len(sizes))
offset = 6 + 16 * len(sizes)
entries = b""
for s, data in zip(sizes, frames):
    entries += struct.pack("<BBBBHHII", s % 256, s % 256, 0, 0, 1, 32, len(data), offset)
    offset += len(data)
open(out, "wb").write(header + entries + b"".join(frames))
PY

# iPhone (also SideStore's source icon)
cp "$work/ios-1024.png" "$repo/ios/App/Resources/Assets.xcassets/AppIcon.appiconset/icon-1024.png"

# Android: legacy icons and the adaptive foreground
res="$repo/android/app/src/main/res"
for d in mdpi hdpi xhdpi xxhdpi xxxhdpi; do
  cp "$work/android-$d.png" "$res/mipmap-$d/ic_launcher.png"
done
mkdir -p "$res/drawable-nodpi"
cp "$work/android-foreground.png" "$res/drawable-nodpi/ic_launcher_foreground.png"
echo "icons written"
