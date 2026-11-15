#!/bin/sh
# Publish an already built static directory with scp. Never create an archive.
set -eu
cd "$(dirname "$0")/.."
target=${1:?Usage: website/deploy.sh user@host [release-name]}
release=${2:-$(date -u +%Y%m%dT%H%M%SZ)}
case "$release" in ''|*[!A-Za-z0-9_-]*) echo 'Release name may use letters, digits, _ and - only.' >&2; exit 1;; esac
public=output/website
test -f "$public/index.html"
test -f "$public/index.en.html"
test -f "$public/assets/guide/voloo-quick-start-en.pdf"
ssh "$target" "set -eu; test ! -e /var/www/voloo/releases/$release; mkdir -p /var/www/voloo/releases/$release"
scp -r "$public/." "$target:/var/www/voloo/releases/$release/"
ssh "$target" "set -eu; test -f /var/www/voloo/releases/$release/index.html; test -f /var/www/voloo/releases/$release/index.en.html; find /var/www/voloo/releases/$release -type d -exec chmod 755 {} +; find /var/www/voloo/releases/$release -type f -exec chmod 644 {} +; ln -s releases/$release /var/www/voloo/current-$release; mv -Tf /var/www/voloo/current-$release /var/www/voloo/current"
echo 'Published https://voloo.librezoo.com/'
