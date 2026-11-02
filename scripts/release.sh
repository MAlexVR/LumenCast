#!/bin/sh
# Signs the packages that `pnpm tauri build` made, writes their checksums and
# prints the command that publishes the release. It publishes nothing itself.
#
#   pnpm tauri build && scripts/release.sh
#
# The signing key is the one `pnpm tauri signer generate` made; its public half
# is in crates/lumencast-update/update.pub.
set -eu
cd "$(dirname "$0")/.."
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
key="${LUMENCAST_UPDATE_KEY:-$HOME/.config/lumencast-release/update.key}"
[ -f "$key" ] || { echo "no signing key at $key" >&2; exit 1; }
bundles=target/release/bundle
out="target/release/release-$version"
rm -rf "$out"
mkdir -p "$out"
count=0
for file in "$bundles"/rpm/*.rpm "$bundles"/deb/*.deb "$bundles"/appimage/*.AppImage; do
    [ -f "$file" ] || continue
    case "$file" in *"$version"*) ;; *) echo "skipping $file (not $version)" >&2; continue ;; esac
    cp "$file" "$out/"
    pnpm tauri signer sign -f "$key" -p "${LUMENCAST_UPDATE_KEY_PASSWORD:-}" "$out/$(basename "$file")" >/dev/null
    count=$((count + 1))
done
[ "$count" -gt 0 ] || { echo "no packages of version $version in $bundles" >&2; exit 1; }
(cd "$out" && sha256sum -- *.rpm *.deb *.AppImage 2>/dev/null > SHA256SUMS.txt)
echo "ready in $out:"
ls -1 "$out"
echo
echo "publish with:"
echo "  gh release create v$version $out/* --repo MAlexVR/LumenCast --title \"LumenCast $version\" --notes-file <(sed -n '/^## \\[$version\\]/,/^## \\[/p' CHANGELOG.md | sed '1d;\$d')"
