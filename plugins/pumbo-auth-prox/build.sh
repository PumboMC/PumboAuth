#!/usr/bin/env bash
# Builds PumboAuth for PumboProx into dist/pumbo-auth.wasm: one file to drop into
# the proxy's plugins/. The manifest (pumbo-auth.yml), the default config
# (assets/config.yml) and the messages (assets/lang/) are built in; the proxy writes
# plugins/pumbo-auth/config.yml and lang/*.yml at the first start.
set -euo pipefail
cd "$(dirname "$0")"
ROOT=$(cd ../.. && pwd)
TARGET=wasm32-wasip2
mkdir -p dist
cargo build --release --target "$TARGET" -p pumbo-auth-prox --target-dir "$ROOT/target/auth-prox"
cp "$ROOT/target/auth-prox/$TARGET/release/pumbo_auth_prox.wasm" dist/pumbo-auth.wasm
ls -l dist/pumbo-auth.wasm
