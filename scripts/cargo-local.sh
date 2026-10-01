#!/bin/sh
# Use standard Rust if present, otherwise the workspace-local toolchain used during development.
set -eu
if command -v cargo >/dev/null 2>&1; then exec cargo "$@"; fi
project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
workspace_tools="${MACRUN_TOOLS_ROOT:-$project_root/../../../.tools}"
export CARGO_HOME="$workspace_tools/cargo"
export RUSTUP_HOME="$workspace_tools/rustup"
export PATH="$CARGO_HOME/bin:$PATH"
exec cargo "$@"
