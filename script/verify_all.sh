#!/usr/bin/env bash
set -euo pipefail

source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

cargo_cmd test --workspace
"$ROOT_DIR/script/build.sh" macos-cli debug
"$ROOT_DIR/script/build.sh" macos-app debug

if command -v rustup >/dev/null 2>&1 && rustup target list --installed | grep -qx "x86_64-pc-windows-gnu"; then
  cargo_cmd check --target x86_64-pc-windows-gnu -p driveck-core -p driveck-win32
else
  echo "Skipping Windows cross-check; install x86_64-pc-windows-gnu to enable it." >&2
fi
