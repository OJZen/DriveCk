#!/usr/bin/env bash
set -euo pipefail

source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

TARGET="${1:-}"
shift || true

OUTPUT_DIR="$ROOT_DIR/target/release"
SNAPSHOT_MODE="auto"

usage() {
  echo "usage: $0 [cli|gtk|win32|macos-cli|macos-app] [--snapshot] [--output-dir DIR]" >&2
}

die() {
  echo "$*" >&2
  exit 2
}

archive_stage_dir() {
  local stage_root="$1"
  local stage_name="$2"
  local archive_path="$3"
  local format="$4"

  case "$format" in
    tar.gz)
      tar -C "$stage_root" -czf "$archive_path" "$stage_name"
      ;;
    zip)
      if command -v ditto >/dev/null 2>&1; then
        ditto -c -k --keepParent "$stage_root/$stage_name" "$archive_path"
      elif command -v zip >/dev/null 2>&1; then
        (
          cd "$stage_root"
          zip -qry "$archive_path" "$stage_name"
        )
      elif command -v tar >/dev/null 2>&1; then
        tar -C "$stage_root" -a -cf "$archive_path" "$stage_name"
      else
        die "zip packaging requires ditto, zip, or tar with zip support."
      fi
      ;;
    *)
      die "unsupported archive format: $format"
      ;;
  esac
}

copy_into_stage() {
  local source_path="$1"
  local destination_path="$2"

  mkdir -p "$(dirname "$destination_path")"
  if [ -d "$source_path" ]; then
    cp -R "$source_path" "$destination_path"
  else
    cp "$source_path" "$destination_path"
  fi
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --snapshot)
      SNAPSHOT_MODE="snapshot"
      ;;
    --output-dir)
      shift
      [ "$#" -gt 0 ] || die "--output-dir requires a directory path."
      OUTPUT_DIR="$1"
      ;;
    *)
      usage
      exit 2
      ;;
  esac
  shift
done

[ -n "$TARGET" ] || {
  usage
  exit 2
}

HOST_OS="$(host_os_id || true)"
HOST_ARCH="$(host_arch_id)"
VERSION="v$(workspace_version)"
VERSION_SUFFIX="$(git_snapshot_suffix "$SNAPSHOT_MODE")"

case "$TARGET" in
  cli)
    PLATFORM_ID="${HOST_OS:-unknown}"
    [ "$PLATFORM_ID" != "macos" ] || die "Use macos-cli to package the native macOS CLI."
    EDITION_ID="cli"
    if [ "$PLATFORM_ID" = "windows" ]; then
      ARCHIVE_FORMAT="zip"
      STAGED_BINARY_NAME="driveck.exe"
      BUILT_BINARY_PATH="$ROOT_DIR/target/release/driveck-cli.exe"
    elif [ "$PLATFORM_ID" = "macos" ]; then
      ARCHIVE_FORMAT="zip"
      STAGED_BINARY_NAME="driveck"
      BUILT_BINARY_PATH="$ROOT_DIR/target/release/driveck-cli"
    else
      ARCHIVE_FORMAT="tar.gz"
      STAGED_BINARY_NAME="driveck"
      BUILT_BINARY_PATH="$ROOT_DIR/target/release/driveck-cli"
    fi
    "$ROOT_DIR/script/build.sh" cli release
    ;;
  gtk)
    [ "${HOST_OS:-}" = "linux" ] || die "gtk packaging requires a Linux host."
    PLATFORM_ID="linux"
    EDITION_ID="gui"
    ARCHIVE_FORMAT="tar.gz"
    STAGED_BINARY_NAME="driveck"
    BUILT_BINARY_PATH="$ROOT_DIR/target/release/driveck"
    "$ROOT_DIR/script/build.sh" gtk release
    ;;
  win32)
    [ "${HOST_OS:-}" = "windows" ] || die "win32 packaging requires a Windows host."
    PLATFORM_ID="windows"
    EDITION_ID="gui"
    ARCHIVE_FORMAT="zip"
    STAGED_BINARY_NAME="DriveCk.exe"
    BUILT_BINARY_PATH="$ROOT_DIR/target/release/driveck-win32.exe"
    "$ROOT_DIR/script/build.sh" win32 release
    ;;
  macos-cli)
    [ "${HOST_OS:-}" = "macos" ] || die "macos-cli packaging requires a macOS host."
    PLATFORM_ID="macos"
    EDITION_ID="cli"
    ARCHIVE_FORMAT="zip"
    STAGED_BINARY_NAME="driveck"
    BUILT_BINARY_PATH="$ROOT_DIR/macos/Build/Release/$MACOS_CLI_NAME"
    "$ROOT_DIR/script/build.sh" macos-cli release
    ;;
  macos-app)
    [ "${HOST_OS:-}" = "macos" ] || die "macos-app packaging requires a macOS host."
    PLATFORM_ID="macos"
    EDITION_ID="gui"
    ARCHIVE_FORMAT="zip"
    STAGED_BINARY_NAME="$MACOS_APP_NAME.app"
    BUILT_BINARY_PATH="$ROOT_DIR/macos/Build/Release/$MACOS_APP_NAME.app"
    "$ROOT_DIR/script/build.sh" macos-app release
    ;;
  *)
    usage
    exit 2
    ;;
esac

if git_worktree_dirty && [ "$SNAPSHOT_MODE" != "snapshot" ]; then
  echo "Packaging from a dirty worktree; archive names stay version-only unless you pass --snapshot." >&2
fi

ARTIFACT_BASENAME="DriveCk-${EDITION_ID}-${PLATFORM_ID}-${HOST_ARCH}-${VERSION}${VERSION_SUFFIX}"
ARCHIVE_PATH="$OUTPUT_DIR/$ARTIFACT_BASENAME.$ARCHIVE_FORMAT"
STAGING_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/driveck-package.XXXXXX")"
STAGE_DIR="$STAGING_ROOT/$ARTIFACT_BASENAME"

cleanup() {
  rm -rf "$STAGING_ROOT"
}
trap cleanup EXIT

mkdir -p "$OUTPUT_DIR" "$STAGE_DIR"
[ -e "$BUILT_BINARY_PATH" ] || die "expected build output was not found: $BUILT_BINARY_PATH"

copy_into_stage "$ROOT_DIR/README.md" "$STAGE_DIR/README.md"
copy_into_stage "$BUILT_BINARY_PATH" "$STAGE_DIR/$STAGED_BINARY_NAME"

case "$TARGET" in
  gtk)
    mkdir -p "$STAGE_DIR/resources/icon"
    copy_into_stage "$ROOT_DIR/resources/icon/linux" "$STAGE_DIR/resources/icon/linux"
    ;;
  macos-app)
    copy_into_stage "$ROOT_DIR/macos/Build/Release/$MACOS_CLI_NAME" "$STAGE_DIR/$MACOS_CLI_NAME"
    ;;
esac

archive_stage_dir "$STAGING_ROOT" "$ARTIFACT_BASENAME" "$ARCHIVE_PATH" "$ARCHIVE_FORMAT"
printf '%s\n' "$ARCHIVE_PATH"
