#!/bin/sh
set -eu
cd "$(dirname "$0")"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT HUP INT TERM
curl --fail --location --silent --show-error \
  https://registry.npmjs.org/@leanprover/infoview/-/infoview-0.14.0.tgz \
  --output "$tmp/infoview.tgz"
printf '%s  %s\n' c71ea9994e86ae2cfbf62db38cd1822a5b9c8d967d8bfa29b079982dc4dec8b3 "$tmp/infoview.tgz" | shasum -a 256 -c -
tar -xzf "$tmp/infoview.tgz" -C "$tmp"
mkdir -p public/vendor/infoview
for file in index.production.min.js index.css codicon.ttf react.production.min.js react-dom.production.min.js react-jsx-runtime.production.min.js; do
  test -s "$tmp/package/dist/$file"
  cp "$tmp/package/dist/$file" "public/vendor/infoview/$file"
done
