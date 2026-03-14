#!/usr/bin/env bash
set -euo pipefail

. "$(dirname "$0")/codex-setup.sh"

cargo fmt --all --check
cargo build --target xtensa-esp32-espidf
