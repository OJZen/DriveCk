#!/usr/bin/env bash
set -euo pipefail

MODE="user"
ARCHIVE_PATH=""

usage() {
  cat >&2 <<'EOF'
usage: ./script/install_linux.sh [--user|--system] <DriveCk-linux-package.tar.gz>

Installs a Linux DriveCk release package into standard user or system paths.

  --user    install into ~/.local (default)
  --system  install into /usr/local (requires root)
EOF
}

die() {
  echo "$*" >&2
  exit 2
}

absolute_path() {
  case "$1" in
    /*) printf '%s\n' "$1" ;;
    *) printf '%s/%s\n' "$PWD" "$1" ;;
  esac
}

ensure_safe_symlink() {
  local link_path="$1"
  local managed_root="$2"

  if [ -L "$link_path" ]; then
    local existing_target
    existing_target="$(readlink "$link_path")"
    case "$existing_target" in
      "$managed_root"/*|"$managed_root")
        return
        ;;
      *)
        die "Refusing to replace unmanaged symlink: $link_path -> $existing_target"
        ;;
    esac
  fi

  if [ -e "$link_path" ]; then
    die "Refusing to overwrite existing path: $link_path"
  fi
}

install_regular_file() {
  local source_path="$1"
  local destination_path="$2"
  local mode="$3"

  install -d "$(dirname "$destination_path")"
  install -m "$mode" "$source_path" "$destination_path"
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --user)
      MODE="user"
      ;;
    --system)
      MODE="system"
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    -*)
      usage
      exit 2
      ;;
    *)
      if [ -n "$ARCHIVE_PATH" ]; then
        die "Only one archive path may be provided."
      fi
      ARCHIVE_PATH="$1"
      ;;
  esac
  shift
done

[ -n "$ARCHIVE_PATH" ] || {
  usage
  exit 2
}

ARCHIVE_PATH="$(absolute_path "$ARCHIVE_PATH")"
[ -f "$ARCHIVE_PATH" ] || die "Archive not found: $ARCHIVE_PATH"

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
ICONS_DIR="${DATA_DIR}/icons"

ARCHIVE_ENTRIES="$(tar -tzf "$ARCHIVE_PATH")"
ROOT_NAME="$(printf '%s\n' "$ARCHIVE_ENTRIES" | awk -F/ 'NF { print $1; exit }')"
[ -n "$ROOT_NAME" ] || die "Could not determine archive root directory."

case "$ROOT_NAME" in
  DriveCk-*-linux-*)
    ;;
  *)
    die "This installer only supports Linux DriveCk release archives."
    ;;
esac

PACKAGE_KIND="cli"
if printf '%s\n' "$ARCHIVE_ENTRIES" | grep -qx "${ROOT_NAME}/resources/icon/linux/com.github.driveck.desktop"; then
  PACKAGE_KIND="gui"
fi

TEMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/driveck-install.XXXXXX")"
cleanup() {
  rm -rf "$TEMP_DIR"
}
trap cleanup EXIT

tar -xzf "$ARCHIVE_PATH" -C "$TEMP_DIR"
EXTRACTED_ROOT="${TEMP_DIR}/${ROOT_NAME}"
[ -d "$EXTRACTED_ROOT" ] || die "Extracted archive root not found: $EXTRACTED_ROOT"
[ -f "$EXTRACTED_ROOT/driveck" ] || die "Package does not contain the driveck executable."

PACKAGE_ROOT="${LIB_ROOT}/${PACKAGE_KIND}"
VERSION_DIR="${PACKAGE_ROOT}/${ROOT_NAME}"
CURRENT_LINK="${PACKAGE_ROOT}/current"

install -d "$PACKAGE_ROOT"
rm -rf "$VERSION_DIR"
install -d "$VERSION_DIR"

install_regular_file "$EXTRACTED_ROOT/driveck" "${VERSION_DIR}/driveck" 755
if [ -f "$EXTRACTED_ROOT/README.md" ]; then
  install_regular_file "$EXTRACTED_ROOT/README.md" "${VERSION_DIR}/README.md" 644
fi

if [ -e "$CURRENT_LINK" ] && [ ! -L "$CURRENT_LINK" ]; then
  die "Refusing to replace non-symlink path: $CURRENT_LINK"
fi
ensure_safe_symlink "$CURRENT_LINK" "$PACKAGE_ROOT"
ln -sfn "$VERSION_DIR" "$CURRENT_LINK"

case "$PACKAGE_KIND" in
  cli)
    CLI_LINK="${BIN_DIR}/driveck"
    install -d "$BIN_DIR"
    ensure_safe_symlink "$CLI_LINK" "$PACKAGE_ROOT"
    ln -sfn "${CURRENT_LINK}/driveck" "$CLI_LINK"
    printf '%s\n' "Installed DriveCk CLI to ${VERSION_DIR}"
    printf '%s\n' "Command: ${CLI_LINK}"
    ;;
  gui)
    DESKTOP_TEMPLATE="${EXTRACTED_ROOT}/resources/icon/linux/com.github.driveck.desktop"
    ICON_SOURCE_DIR="${EXTRACTED_ROOT}/resources/icon/linux/hicolor"
    GUI_LINK="${BIN_DIR}/driveck-gui"
    [ -f "$DESKTOP_TEMPLATE" ] || die "GUI package is missing its desktop entry."
    [ -d "$ICON_SOURCE_DIR" ] || die "GUI package is missing its icon theme assets."

    install -d "$BIN_DIR" "$APPLICATIONS_DIR" "${ICONS_DIR}/hicolor"
    ensure_safe_symlink "$GUI_LINK" "$PACKAGE_ROOT"
    ln -sfn "${CURRENT_LINK}/driveck" "$GUI_LINK"
    cp -R "${ICON_SOURCE_DIR}/." "${ICONS_DIR}/hicolor/"

    DESKTOP_FILE="${APPLICATIONS_DIR}/com.github.driveck.desktop"
    sed "s|^Exec=.*|Exec=${CURRENT_LINK}/driveck|" "$DESKTOP_TEMPLATE" >"${TEMP_DIR}/com.github.driveck.desktop"
    install_regular_file "${TEMP_DIR}/com.github.driveck.desktop" "$DESKTOP_FILE" 644

    if command -v update-desktop-database >/dev/null 2>&1; then
      update-desktop-database "$APPLICATIONS_DIR" >/dev/null 2>&1 || true
    fi
    if command -v gtk-update-icon-cache >/dev/null 2>&1; then
      gtk-update-icon-cache -q -t -f "${ICONS_DIR}/hicolor" >/dev/null 2>&1 || true
    fi

    printf '%s\n' "Installed DriveCk GUI to ${VERSION_DIR}"
    printf '%s\n' "Desktop entry: ${DESKTOP_FILE}"
    printf '%s\n' "Command: ${GUI_LINK} (starts the GUI by default and accepts CLI arguments)"
    ;;
esac
