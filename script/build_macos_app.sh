#!/usr/bin/env bash
set -euo pipefail

exec "$(dirname "${BASH_SOURCE[0]}")/build.sh" "macos-app" "${1:-Debug}"
