#!/bin/sh
# Stable local signing: retain the same bundle identity across updates.
set -eu
project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
bundle=${1:-"$project_root/desktop/src-tauri/target/release/bundle/macos/Macrun Desktop.app"}
identity=${MACRUN_SIGNING_IDENTITY:-${APPLE_SIGNING_IDENTITY:-}}
if [ -z "$identity" ]; then
    identities=$(security find-identity -v -p codesigning | sed -n 's/.*"\(Developer ID Application: [^"]*\)".*/\1/p')
    count=$(printf '%s\n' "$identities" | awk 'NF { n++ } END { print n+0 }')
    if [ "$count" != 1 ]; then
        echo 'Set MACRUN_SIGNING_IDENTITY to a Developer ID Application identity in your Keychain.' >&2
        exit 1
    fi
    identity=$identities
fi
if [ "$identity" = '-' ]; then
    echo 'Ad-hoc signing is not suitable for installed updates: permissions are tied to the old binary.' >&2
    exit 1
fi
test -d "$bundle" || { echo 'Build the release application first.' >&2; exit 1; }
codesign --force --sign "$identity" --options runtime --timestamp "$bundle/Contents/MacOS/macrun"
codesign --force --sign "$identity" --options runtime --timestamp "$bundle"
codesign --verify --deep --strict "$bundle"
echo 'Signed application with a stable identity. Notarization is a separate distribution step.'
