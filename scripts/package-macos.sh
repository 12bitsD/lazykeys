#!/usr/bin/env bash
set -euo pipefail

version="${1:?usage: package-macos.sh <version> [output-directory]}"
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
output="${2:-$root/release}"
target="aarch64-apple-darwin"
bundle="$root/src-tauri/target/$target/release/bundle"
app="$bundle/macos/LazyKeys.app"
binary="$app/Contents/MacOS/lazykeys"
source_dmg="$bundle/dmg/LazyKeys_${version}_aarch64.dmg"
app_zip="LazyKeys_${version}_arm64.app.zip"
dmg="LazyKeys_${version}_arm64.dmg"

[[ -d "$app" ]] || { echo "missing App bundle: $app" >&2; exit 1; }
[[ -x "$binary" ]] || { echo "missing App executable: $binary" >&2; exit 1; }
[[ -f "$source_dmg" ]] || { echo "missing DMG: $source_dmg" >&2; exit 1; }
[[ "$(file "$binary")" == *arm64* ]] || { echo "App executable is not arm64" >&2; exit 1; }

mkdir -p "$output"
rm -f "$output/$app_zip" "$output/$dmg" "$output/SHA256SUMS.txt"
ditto -c -k --sequesterRsrc --keepParent "$app" "$output/$app_zip"
cp "$source_dmg" "$output/$dmg"

unzip -t "$output/$app_zip" > /dev/null
grep -qx "LazyKeys.app/Contents/MacOS/lazykeys" <<< "$(zipinfo -1 "$output/$app_zip")"
diskutil image info "$output/$dmg" > /dev/null

(
  cd "$output"
  shasum -a 256 "$app_zip" "$dmg" > SHA256SUMS.txt
  shasum -a 256 -c SHA256SUMS.txt
)
ls -lh "$output/$app_zip" "$output/$dmg" "$output/SHA256SUMS.txt"
