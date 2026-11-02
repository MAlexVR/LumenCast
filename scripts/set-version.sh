#!/bin/sh
# Sets the version of LumenCast in every place that carries it:
#   scripts/set-version.sh 1.2.3
set -eu
version="${1:?usage: set-version.sh MAJOR.MINOR.PATCH[-pre]}"
case "$version" in
  [0-9]*.[0-9]*.[0-9]*) ;;
  *) echo "not a version: $version" >&2; exit 2 ;;
esac
cd "$(dirname "$0")/.."
sed -i "0,/^version = \".*\"/s//version = \"$version\"/" Cargo.toml
sed -i "0,/\"version\": \".*\"/s//\"version\": \"$version\"/" package.json ui/package.json src-tauri/tauri.conf.json
cargo update --workspace --offline >/dev/null 2>&1 || cargo update --workspace >/dev/null
echo "LumenCast is now $version; add its entry to CHANGELOG.md and tag it v$version."
