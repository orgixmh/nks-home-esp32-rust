#!/usr/bin/env bash
set -euo pipefail

. "$(dirname "$0")/codex-setup.sh"

PORT="${ESP_PORT:-/dev/ttyUSB0}"
BAUD="${ESP_BAUD:-115200}"

cargo espflash flash --monitor --port "$PORT" --baud "$BAUD" --target xtensa-esp32-espidf
