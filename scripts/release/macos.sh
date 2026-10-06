#!/usr/bin/env bash
# Run on an ephemeral macOS CI runner. Secret values must never be traced.
set -euo pipefail
root=$(cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$root"
version=${1:?version required}
target=${2:?Rust target required}
case "$target" in
  aarch64-apple-darwin) arch=arm64 ;;
  x86_64-apple-darwin) arch=x86_64 ;;
  *) echo 'Unsupported macOS target' >&2; exit 1 ;;
esac
for name in APPLE_CERTIFICATE APPLE_CERTIFICATE_PASSWORD APPLE_SIGNING_IDENTITY APPLE_API_ISSUER APPLE_API_KEY APPLE_API_PRIVATE_KEY; do
  [[ -n "${!name:-}" ]] || { echo "Missing secret: $name" >&2; exit 1; }
done
[[ "$APPLE_SIGNING_IDENTITY" == 'Developer ID Application: '* ]] || { echo 'Expected Developer ID Application identity' >&2; exit 1; }
python3 scripts/release/release.py check --tag "v$version"
umask 077
temp=$(mktemp -d)
keychain="$temp/release.keychain-db"
cleanup() {
  if [[ -d "$temp/mount" ]]; then hdiutil detach "$temp/mount" >/dev/null 2>&1 || true; fi
  security delete-keychain "$keychain" >/dev/null 2>&1 || true
  rm -rf "$temp"
}
trap cleanup EXIT
printf '%s' "$APPLE_CERTIFICATE" | base64 --decode > "$temp/certificate.p12"
printf '%s' "$APPLE_API_PRIVATE_KEY" > "$temp/notary.p8"
keychain_password=$(openssl rand -hex 24)
security create-keychain -p "$keychain_password" "$keychain"
security unlock-keychain -p "$keychain_password" "$keychain"
security set-keychain-settings -lut 7200 "$keychain"
security import "$temp/certificate.p12" -k "$keychain" -P "$APPLE_CERTIFICATE_PASSWORD" -T /usr/bin/codesign
security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$keychain_password" "$keychain" >/dev/null
# Only alters the disposable runner, never a developer's login keychain.
security list-keychains -d user -s "$keychain"
# Secrets remain inside the 0700 temporary directory; distributed bundles must
# be readable/executable by users other than the CI runner's numeric UID.
umask 022
export MACOSX_DEPLOYMENT_TARGET=12.0
# Keep notarization in this script so both app and final DMG have verified tickets.
(cd desktop && env -u APPLE_CERTIFICATE -u APPLE_CERTIFICATE_PASSWORD -u APPLE_API_ISSUER -u APPLE_API_KEY -u APPLE_API_KEY_PATH -u APPLE_API_PRIVATE_KEY -u APPLE_ID -u APPLE_PASSWORD npm run desktop:build -- --target "$target" --bundles app --config '{"bundle":{"macOS":{"hardenedRuntime":true}}}')
app="$root/desktop/src-tauri/target/$target/release/bundle/macos/Macrun Desktop.app"
for binary in "$app/Contents/MacOS/macrun" "$app/Contents/MacOS/macrun-desktop"; do
  [[ "$(lipo -archs "$binary")" == "$arch" ]] || { echo "Wrong architecture: $binary (expected $arch)" >&2; exit 1; }
done
codesign --verify --deep --strict "$app"
notarize() {
  local artifact=$1 result=$2
  xcrun notarytool submit "$artifact" --key "$temp/notary.p8" --key-id "$APPLE_API_KEY" --issuer "$APPLE_API_ISSUER" --wait --timeout 30m --output-format json > "$result"
  if ! python3 - "$result" <<'PY'
import json, sys
result = json.load(open(sys.argv[1]))
if result.get("status") != "Accepted":
    raise SystemExit(f"Notarization failed: id={result.get('id')}, status={result.get('status')}")
PY
  then
    local submission
    submission=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["id"])' "$result")
    xcrun notarytool log "$submission" --key "$temp/notary.p8" --key-id "$APPLE_API_KEY" --issuer "$APPLE_API_ISSUER" "${result%.json}-log.json" || true
    return 1
  fi
}
mkdir -p dist/notary dist/release
ditto -c -k --keepParent "$app" "$temp/app.zip"
notarize "$temp/app.zip" "dist/notary/app-$arch.json"
xcrun stapler staple "$app"
xcrun stapler validate "$app"
spctl --assess --type execute --verbose=2 "$app"
mkdir "$temp/image"
ditto "$app" "$temp/image/Macrun Desktop.app"
ln -s /Applications "$temp/image/Applications"
dmg="$root/dist/release/Macrun-Desktop_${version}_macos_${arch}.dmg"
hdiutil create -volname 'Macrun Desktop' -srcfolder "$temp/image" -ov -format UDZO "$dmg"
codesign --sign "$APPLE_SIGNING_IDENTITY" --timestamp "$dmg"
notarize "$dmg" "dist/notary/dmg-$arch.json"
xcrun stapler staple "$dmg"
xcrun stapler validate "$dmg"
codesign --verify --strict "$dmg"
spctl --assess --type open --context context:primary-signature --verbose=2 "$dmg"
hdiutil verify "$dmg"
mkdir "$temp/mount"
hdiutil attach "$dmg" -readonly -nobrowse -mountpoint "$temp/mount"
codesign --verify --deep --strict "$temp/mount/Macrun Desktop.app"
xcrun stapler validate "$temp/mount/Macrun Desktop.app"
hdiutil detach "$temp/mount"
