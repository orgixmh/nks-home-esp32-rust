#!/usr/bin/env bash
set -euo pipefail

PORT="${1:-/dev/ttyUSB0}"

cd "$(dirname "$0")/.."

source ~/.cargo/env
source ~/export-esp.sh

export ESPFLASH_BAUD=460800
export MONITOR_BAUD=115200
cargo espflash flash --monitor --port "$PORT"
