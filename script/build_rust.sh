#!/usr/bin/env bash
set -euo pipefail

source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

TARGET="${1:-workspace}"
PROFILE="${2:-debug}"

usage() {
  echo "usage: $0 [workspace|core|cli|ffi|gtk|win32] [debug|release]" >&2
}

case "$PROFILE" in
  debug|release) ;;
  *)
    usage
    exit 2
    ;;
esac

run_build() {
  if [ "$PROFILE" = "release" ]; then
    cargo_cmd build "$@" --release
  else
    cargo_cmd build "$@"
  fi
}

case "$TARGET" in
  workspace)
    run_build --workspace
    ;;
  core)
    run_build -p driveck-core
    ;;
  cli)
    run_build -p driveck-cli
    ;;
  ffi)
    run_build -p driveck-ffi
    ;;
  gtk)
    run_build -p driveck-gtk
    ;;
  win32)
    run_build -p driveck-win32
    ;;
  *)
    usage
    exit 2
    ;;
esac
