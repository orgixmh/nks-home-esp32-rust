#!/usr/bin/env bash
set -euo pipefail

if [ -f "$HOME/export-esp.sh" ]; then
  . "$HOME/export-esp.sh"
else
  echo "ERROR: $HOME/export-esp.sh not found"
  exit 1
fi

command -v rustc >/dev/null 2>&1 || { echo "rustc missing"; exit 1; }
command -v cargo >/dev/null 2>&1 || { echo "cargo missing"; exit 1; }
command -v ldproxy >/dev/null 2>&1 || { echo "ldproxy missing"; exit 1; }

rustc --version
cargo --version
