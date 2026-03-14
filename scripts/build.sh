#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

source ~/.cargo/env
source ~/export-esp.sh

cargo build
