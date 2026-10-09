#!/usr/bin/env bash
# Builds PumboAuth for every supported Pumpkin release into dist/ at the
# workspace root:
#   dist/PumboAuth-26.3.wasm  -> Pumpkin 0.2.0+26.3-26.51     (Minecraft 26.3)
#   dist/PumboAuth-26.2.wasm  -> Pumpkin 0.1.0-dev+26.2-26.45 (Minecraft 26.2)
set -euo pipefail
cd "$(dirname "$0")/../.."
TARGET=wasm32-wasip2
PKG=pumbo-auth-pumpkin
OUT=pumbo_auth_pumpkin.wasm
mkdir -p dist

cargo build -p "$PKG" --release --target "$TARGET"
cp "target/$TARGET/release/$OUT" dist/PumboAuth-26.3.wasm

cargo build -p "$PKG" --release --target "$TARGET" --no-default-features --features mc262 --target-dir target/mc262
cp "target/mc262/$TARGET/release/$OUT" dist/PumboAuth-26.2.wasm

ls -l dist/PumboAuth-*.wasm
