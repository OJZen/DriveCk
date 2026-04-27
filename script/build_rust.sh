#!/usr/bin/env bash
set -euo pipefail

exec "$(dirname "${BASH_SOURCE[0]}")/build.sh" "${1:-workspace}" "${2:-debug}"
