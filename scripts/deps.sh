#!/bin/sh
# Installs Rust when missing; fetches exact Cargo.lock dependencies.
set -eu
project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_root"
. "$project_root/scripts/rust-env.sh"
if ! command -v cc >/dev/null 2>&1; then
    echo 'A native C compiler/linker is required.' >&2
    echo 'macOS: xcode-select --install; Debian/Ubuntu: sudo apt-get install build-essential' >&2
    exit 1
fi
if [ "$(uname -s)" = Darwin ] && ! xcode-select -p >/dev/null 2>&1; then
    echo 'Install Apple developer tools first: xcode-select --install' >&2
    exit 1
fi
if ! command -v cargo >/dev/null 2>&1; then
    command -v curl >/dev/null 2>&1 || { echo 'Install curl first.' >&2; exit 1; }
    mkdir -p "$project_root/.local"
    echo "Installing Rust under $CARGO_HOME (shell profile is not modified)"
    curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs -o "$project_root/.local/rustup-init.sh"
    sh "$project_root/.local/rustup-init.sh" -y --no-modify-path --profile minimal --default-toolchain "${RUST_VERSION:-1.98.0}"
fi
if command -v rustup >/dev/null 2>&1; then
    rustup component add rustfmt clippy
else
    cargo fmt --version >/dev/null 2>&1 && cargo clippy --version >/dev/null 2>&1 || {
        echo 'This Rust installation has no rustup. Install rustfmt and clippy using its package manager.' >&2
        exit 1
    }
fi
cargo fetch --locked
cargo --version
