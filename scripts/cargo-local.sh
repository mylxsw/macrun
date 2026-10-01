#!/bin/sh
set -eu
project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
. "$project_root/scripts/rust-env.sh"
if ! command -v cargo >/dev/null 2>&1; then
    echo 'Rust is not installed. Run: make deps' >&2
    exit 1
fi
exec cargo "$@"
