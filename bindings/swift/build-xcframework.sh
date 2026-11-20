#!/usr/bin/env bash
# Builds the engine for iOS (device and Apple-silicon simulator) as BrushEngine.xcframework, and copies it with the
# Swift wrapper into the iOS app (../../../mobile/Vendor/BrushEngine). Needs Rust with the aarch64-apple-ios and
# simulator targets (rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios) and Xcode.
# The build is checked in, so the app builds without a Rust toolchain, as the web app vendors the wasm build.
set -euo pipefail
cd "$(dirname "$0")"
ROOT=../..
OUT=${1:-../../../mobile/Vendor/BrushEngine}
export IPHONEOS_DEPLOYMENT_TARGET=17.0
for target in aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios; do
  cargo build --release -p brush-ffi --target "$target"
done
STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
mkdir -p "$STAGE/headers" "$STAGE/ios" "$STAGE/sim"
# Debug info is most of the archive's size and none of the app's; strip it (global symbols stay for linking).
xcrun strip -S -o "$STAGE/ios/libbrush_ffi.a" "$ROOT/target/aarch64-apple-ios/release/libbrush_ffi.a" 2>/dev/null
# The simulator slice is universal (Apple silicon and Intel Macs).
xcrun lipo -create -output "$STAGE/sim/universal.a" \
  "$ROOT/target/aarch64-apple-ios-sim/release/libbrush_ffi.a" "$ROOT/target/x86_64-apple-ios/release/libbrush_ffi.a"
xcrun strip -S -o "$STAGE/sim/libbrush_ffi.a" "$STAGE/sim/universal.a" 2>/dev/null
rm "$STAGE/sim/universal.a"
cp include/brush_engine.h include/module.modulemap "$STAGE/headers/"
rm -rf "$OUT/BrushEngine.xcframework"
mkdir -p "$OUT"
xcodebuild -create-xcframework \
  -library "$STAGE/ios/libbrush_ffi.a" -headers "$STAGE/headers" \
  -library "$STAGE/sim/libbrush_ffi.a" -headers "$STAGE/headers" \
  -output "$OUT/BrushEngine.xcframework" >/dev/null
cp Sources/BrushEngine.swift "$OUT/BrushEngine.swift"
echo "Built $OUT/BrushEngine.xcframework"
