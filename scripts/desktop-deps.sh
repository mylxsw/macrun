#!/bin/sh
set -eu
project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_root"
if [ "$(uname -s)" != Darwin ]; then
    echo 'Macrun Desktop currently requires macOS.' >&2
    exit 1
fi
command -v node >/dev/null 2>&1 && command -v npm >/dev/null 2>&1 || {
    echo 'Install Node.js 22.12+ (including npm), then run make again.' >&2
    exit 1
}
node -e 'const [major,minor]=process.versions.node.split(".").map(Number); if(major<22 || (major===22 && minor<12)){console.error("Node.js 22.12+ is required.");process.exit(1)}'
if ! xcrun --find clang >/dev/null 2>&1; then
    echo 'Install Apple developer tools with: xcode-select --install; then run make again.' >&2
    exit 1
fi
. "$project_root/scripts/rust-env.sh"
if ! command -v cargo >/dev/null 2>&1; then
    ./scripts/deps.sh
fi
cd desktop
if [ ! -x node_modules/.bin/tauri ] || [ ! -x node_modules/.bin/vite ] ||
   ! cmp -s package-lock.json node_modules/.macrun-package-lock.json ||
   ! cmp -s package.json node_modules/.macrun-package.json; then
    npm ci --include=dev --cache "${npm_config_cache:-$project_root/.local/npm-cache}"
    cp package-lock.json node_modules/.macrun-package-lock.json
    cp package.json node_modules/.macrun-package.json
else
    echo 'Desktop dependencies are ready.'
fi
