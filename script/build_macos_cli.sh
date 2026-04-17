#!/usr/bin/env bash
set -euo pipefail

source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

CONFIGURATION="${1:-Debug}"

case "$CONFIGURATION" in
  Debug|Release) ;;
  *)
    echo "usage: $0 [Debug|Release]" >&2
    exit 2
    ;;
esac

xcodebuild_cmd -scheme DriveCkMacCLI -configuration "$CONFIGURATION" build
