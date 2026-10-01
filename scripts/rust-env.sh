#!/bin/sh
# Shared by cargo-local.sh and deps.sh. Caller supplies project_root.
# Explicit tool roots win; otherwise reuse an installed toolchain, then local storage.
if [ -n "${MACRUN_TOOLS_ROOT:-}" ]; then
    export CARGO_HOME="$MACRUN_TOOLS_ROOT/cargo"
    export RUSTUP_HOME="$MACRUN_TOOLS_ROOT/rustup"
    export PATH="$CARGO_HOME/bin:$PATH"
elif command -v cargo >/dev/null 2>&1; then
    :
elif [ -x "${CARGO_HOME:-$HOME/.cargo}/bin/cargo" ]; then
    export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"
elif [ -x "$project_root/../../../.tools/cargo/bin/cargo" ]; then
    export CARGO_HOME="$project_root/../../../.tools/cargo"
    export RUSTUP_HOME="$project_root/../../../.tools/rustup"
    export PATH="$CARGO_HOME/bin:$PATH"
else
    export CARGO_HOME="${CARGO_HOME:-$project_root/.local/tools/cargo}"
    export RUSTUP_HOME="${RUSTUP_HOME:-$project_root/.local/tools/rustup}"
    export PATH="$CARGO_HOME/bin:$PATH"
fi
