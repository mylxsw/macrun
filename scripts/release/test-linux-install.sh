#!/usr/bin/env bash
# Destructive package lifecycle test: run only inside a disposable container.
set -euo pipefail
[[ -f /.dockerenv ]]
: "${RELEASE_VERSION:?}" "${RELEASE_ARCH:?}"
[[ "$(dpkg --print-architecture)" == "$RELEASE_ARCH" ]]
apt-get update
apt-get install -y --no-install-recommends python3 ca-certificates
package="/src/dist/release/macrun_${RELEASE_VERSION}_${RELEASE_ARCH}.deb"
apt-get install -y "$package"
[[ "$(macrun --version)" == "macrun $RELEASE_VERSION" ]]
python3 /src/scripts/smoke.py /usr/bin/macrun
mkdir -p /var/lib/macrun
printf 'preserve\n' > /var/lib/macrun/release-test
apt-get install -y --reinstall "$package"
[[ "$(cat /var/lib/macrun/release-test)" == preserve ]]
apt-get purge -y macrun
[[ "$(cat /var/lib/macrun/release-test)" == preserve ]]
[[ ! -e /usr/bin/macrun ]]
mkdir /tmp/macrun-archive
tar -xzf "/src/dist/release/macrun_${RELEASE_VERSION}_linux_${RELEASE_ARCH}.tar.gz" -C /tmp/macrun-archive
[[ "$(/tmp/macrun-archive/macrun --version)" == "macrun $RELEASE_VERSION" ]]
