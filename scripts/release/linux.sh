#!/usr/bin/env bash
set -euo pipefail
root=$(cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$root"
version=${1:?version required}
arch=${2:?amd64 or arm64 required}
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]
[[ "$arch" == amd64 || "$arch" == arm64 ]]
[[ "$(dpkg --print-architecture)" == "$arch" ]]
python3 scripts/release/release.py check --tag "v$version"
cargo build --locked --release
[[ "$(target/release/macrun --version)" == "macrun $version" ]]
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
mkdir -p dist/release "$stage/package/usr/bin" "$stage/package/usr/share/doc/macrun" "$stage/debian" "$stage/archive"
install -m755 target/release/macrun "$stage/package/usr/bin/macrun"
install -m644 LICENSE "$stage/package/usr/share/doc/macrun/copyright"
install -m644 docs/release/linux-install.md "$stage/package/usr/share/doc/macrun/README.md"
install -m644 deploy/macrun-server.service "$stage/package/usr/share/doc/macrun/macrun-server.service.example"
cat > "$stage/debian/control" <<'EOF'
Source: macrun
Section: utils
Priority: optional
Maintainer: Macrun contributors <noreply@github.com>

Package: macrun
Architecture: any
Description: Remote commands, files, synchronization and MCP over QUIC
EOF
depends=$(cd "$stage" && dpkg-shlibdeps -O -e "$root/target/release/macrun")
depends=${depends#shlibs:Depends=}
[[ -n "$depends" && "$depends" != *$'\n'* ]]
mkdir -p "$stage/package/DEBIAN"
cat > "$stage/package/DEBIAN/control" <<EOF
Package: macrun
Version: $version
Architecture: $arch
Maintainer: Macrun contributors <noreply@github.com>
Section: utils
Priority: optional
Depends: $depends
Description: Remote commands, files, synchronization and MCP over QUIC
EOF
dpkg-deb --root-owner-group --build "$stage/package" "dist/release/macrun_${version}_${arch}.deb"
cp "$stage/package/usr/bin/macrun" "$stage/archive/"
cp "$stage/package/usr/share/doc/macrun/"* "$stage/archive/"
tar -czf "dist/release/macrun_${version}_linux_${arch}.tar.gz" -C "$stage/archive" .
# Exercise the archive as delivered, independently of the build-tree executable.
mkdir "$stage/unpack"
tar -xzf "dist/release/macrun_${version}_linux_${arch}.tar.gz" -C "$stage/unpack"
[[ "$("$stage/unpack/macrun" --version)" == "macrun $version" ]]
