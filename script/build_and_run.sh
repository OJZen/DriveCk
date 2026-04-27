#!/usr/bin/env bash
set -euo pipefail

source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

MODE="${1:-run}"
APP_BUNDLE="$ROOT_DIR/macos/Build/Debug/${MACOS_APP_NAME}.app"
APP_BINARY="$APP_BUNDLE/Contents/MacOS/$MACOS_APP_NAME"

build_app() {
  "$ROOT_DIR/script/build.sh" macos-app debug
}

open_app() {
  /usr/bin/open "$APP_BUNDLE"
}

verify_running() {
  sleep 1
  pgrep -x "$MACOS_APP_NAME" >/dev/null
}

ensure_app_not_running() {
  if app_is_running; then
    echo "$MACOS_APP_NAME is already running. Close it manually before rebuilding so validation cannot be interrupted mid-run." >&2
    exit 2
  fi
}

case "$MODE" in
  build|--build)
    ensure_app_not_running
    build_app
    ;;
  run)
    ensure_app_not_running
    build_app
    open_app
    ;;
  --debug|debug)
    ensure_app_not_running
    build_app
    lldb -- "$APP_BINARY"
    ;;
  --logs|logs)
    ensure_app_not_running
    build_app
    open_app
    /usr/bin/log stream --info --style compact --predicate "process == \"$MACOS_APP_NAME\""
    ;;
  --telemetry|telemetry)
    ensure_app_not_running
    build_app
    open_app
    /usr/bin/log stream --info --style compact --predicate "subsystem == \"$MACOS_BUNDLE_ID\""
    ;;
  --verify|verify)
    ensure_app_not_running
    build_app
    open_app
    verify_running
    ;;
  *)
    echo "usage: $0 [build|run|--debug|--logs|--telemetry|--verify]" >&2
    exit 2
    ;;
esac
