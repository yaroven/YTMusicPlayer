#!/bin/bash
# Runs the iPhone app's Player (queue, gapless, radio, history) on this Mac,
# muted, against live YouTube: AVFoundation behaves the same on macOS.
# Needs only Command Line Tools (no Xcode). Usage: ios/player-check.sh
set -euo pipefail
cd "$(dirname "$0")"
(cd YTMKit && swift build)
out=$(mktemp -d)
swiftc -swift-version 5 -target "$(uname -m)-apple-macos14" -I YTMKit/.build/debug/Modules \
  -o "$out/player-check" PlayerCheck/main.swift \
  App/Sources/Player.swift App/Sources/ChunkedLoader.swift App/Sources/Connectivity.swift App/Sources/Library.swift App/Sources/Navigation.swift App/Sources/Keychain.swift App/Sources/Artwork.swift \
  YTMKit/.build/debug/YTMKit.build/*.o
mkdir -p "$out/home"
HOME="$out/home" "$out/player-check" | tee "$out/log"
rm -rf "$out/home"
! grep -q FAIL "$out/log"
