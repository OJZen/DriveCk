#!/bin/bash

set -euo pipefail

PROJECT_ROOT="$(cd "${PROJECT_DIR}/.." && pwd)"
RUST_TARGET_DIR="${PROJECT_DIR}/Build/RustTarget"
OUTPUT_DIR="${PROJECT_DIR}/Build/Rust/${CONFIGURATION}"

if command -v cargo >/dev/null 2>&1; then
  CARGO_BIN="$(command -v cargo)"
elif [[ -x "${HOME}/.cargo/bin/cargo" ]]; then
  CARGO_BIN="${HOME}/.cargo/bin/cargo"
else
  echo "error: cargo not found. Install Rust and ensure cargo is on PATH." >&2
  exit 1
fi

mkdir -p "${RUST_TARGET_DIR}" "${OUTPUT_DIR}"

PROFILE="debug"
BUILD_ARGS=(build -p driveck-ffi --target-dir "${RUST_TARGET_DIR}")
if [[ "${CONFIGURATION}" == "Release" ]]; then
  PROFILE="release"
  BUILD_ARGS+=(--release)
fi

pushd "${PROJECT_ROOT}" >/dev/null
"${CARGO_BIN}" "${BUILD_ARGS[@]}"
popd >/dev/null

cp "${RUST_TARGET_DIR}/${PROFILE}/libdriveck_ffi.a" "${OUTPUT_DIR}/libdriveck_ffi.a"
