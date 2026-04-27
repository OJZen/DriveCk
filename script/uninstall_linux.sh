#!/usr/bin/env bash
set -euo pipefail

MODE="user"
TARGET="all"

usage() {
  cat >&2 <<'EOF'
usage: ./script/uninstall_linux.sh [--user|--system] [--gui|--cli|--all]

Removes DriveCk Linux installs from the standard user or system locations.

  --user    remove from ~/.local (default)
  --system  remove from /usr/local (requires root)
  --gui     remove the GUI installation only
  --cli     remove the CLI installation only
  --all     remove both GUI and CLI installations (default)
EOF
}

die() {
  echo "$*" >&2
  exit 2
}

warn() {
  echo "$*" >&2
}

remove_managed_symlink() {
  local link_path="$1"
  local managed_root="$2"

  if [ -L "$link_path" ]; then
    local existing_target
    existing_target="$(readlink "$link_path")"
    case "$existing_target" in
      "$managed_root"/*|"$managed_root")
        rm -f "$link_path"
        ;;
      *)
        warn "Skipping unmanaged symlink: $link_path -> $existing_target"
        ;;
    esac
  elif [ -e "$link_path" ]; then
    warn "Skipping unmanaged path: $link_path"
  fi
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --user)
      MODE="user"
      ;;
    --system)
      MODE="system"
      ;;
    --gui)
      TARGET="gui"
      ;;
    --cli)
      TARGET="cli"
      ;;
    --all)
      TARGET="all"
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      usage
      exit 2
      ;;
  esac
  shift
done

if [ "$MODE" = "system" ] && [ "${EUID:-$(id -u)}" -ne 0 ]; then
  die "--system requires root. Re-run with sudo or as root."
fi

if [ "$MODE" = "user" ]; then
  BIN_DIR="${HOME}/.local/bin"
  DATA_DIR="${HOME}/.local/share"
  LIB_ROOT="${HOME}/.local/lib/driveck"
else
  BIN_DIR="/usr/local/bin"
  DATA_DIR="/usr/local/share"
  LIB_ROOT="/usr/local/lib/driveck"
fi

APPLICATIONS_DIR="${DATA_DIR}/applications"
ICONS_DIR="${DATA_DIR}/icons/hicolor"
REMOVED_ANY=0

remove_cli() {
  local cli_root="${LIB_ROOT}/cli"
  remove_managed_symlink "${BIN_DIR}/driveck" "$cli_root"
  if [ -d "$cli_root" ]; then
    rm -rf "$cli_root"
    REMOVED_ANY=1
  fi
}

remove_gui() {
  local gui_root="${LIB_ROOT}/gui"
  local desktop_file="${APPLICATIONS_DIR}/com.github.driveck.desktop"

  remove_managed_symlink "${BIN_DIR}/driveck-gui" "$gui_root"
  if [ -f "$desktop_file" ] && grep -q "${gui_root}/current/driveck" "$desktop_file"; then
    rm -f "$desktop_file"
    REMOVED_ANY=1
  elif [ -e "$desktop_file" ]; then
    warn "Skipping unmanaged desktop entry: $desktop_file"
  fi

  if [ -d "$ICONS_DIR" ]; then
    local rel
    for rel in \
      16x16/apps/com.github.driveck.png \
      24x24/apps/com.github.driveck.png \
      32x32/apps/com.github.driveck.png \
      48x48/apps/com.github.driveck.png \
      64x64/apps/com.github.driveck.png \
      128x128/apps/com.github.driveck.png \
      256x256/apps/com.github.driveck.png \
      512x512/apps/com.github.driveck.png \
      scalable/apps/com.github.driveck.svg
    do
      if [ -f "${ICONS_DIR}/${rel}" ]; then
        rm -f "${ICONS_DIR}/${rel}"
        REMOVED_ANY=1
      fi
    done
  fi

  if [ -d "$gui_root" ]; then
    rm -rf "$gui_root"
    REMOVED_ANY=1
  fi

  if command -v update-desktop-database >/dev/null 2>&1 && [ -d "$APPLICATIONS_DIR" ]; then
    update-desktop-database "$APPLICATIONS_DIR" >/dev/null 2>&1 || true
  fi
  if command -v gtk-update-icon-cache >/dev/null 2>&1 && [ -d "$ICONS_DIR" ]; then
    gtk-update-icon-cache -q -t -f "$ICONS_DIR" >/dev/null 2>&1 || true
  fi
}

case "$TARGET" in
  cli)
    remove_cli
    ;;
  gui)
    remove_gui
    ;;
  all)
    remove_gui
    remove_cli
    ;;
esac

if [ "$REMOVED_ANY" -eq 1 ]; then
  printf '%s\n' "Removed DriveCk ${TARGET} install from ${MODE} scope."
else
  printf '%s\n' "No managed DriveCk ${TARGET} install was found in ${MODE} scope."
fi
