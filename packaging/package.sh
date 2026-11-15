#!/usr/bin/env bash
# Package a release build of Vikui for one target into dist/.
#
#   packaging/package.sh <rust-target> <version>
#
# Expects `cargo build --release --target <rust-target>` to have run. Produces
#   macOS:   dist/vikui-<version>-macos-<arch>.zip     (Vikui.app, ad-hoc signed)
#   Linux:   dist/vikui-<version>-linux-<arch>.tar.gz
#   Windows: dist/vikui-<version>-windows-<arch>.zip
set -euo pipefail

target="${1:?usage: package.sh <rust-target> <version>}"
version="${2:?usage: package.sh <rust-target> <version>}"
version="${version#v}"

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
release="$root/target/$target/release"
dist="$root/dist"
stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT
mkdir -p "$dist"

case "$target" in
  x86_64-*) arch=x64 ;;
  aarch64-*) arch=arm64 ;;
  *) echo "unsupported target: $target" >&2; exit 1 ;;
esac

case "$target" in
  *-apple-darwin)
    name="vikui-$version-macos-$arch"
    app="$stage/Vikui.app"
    mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
    cp "$release/vikui" "$app/Contents/MacOS/vikui"
    cp "$root/packaging/icon/vikui.icns" "$app/Contents/Resources/vikui.icns"
    sed "s/@VERSION@/$version/g" "$root/packaging/macos/Info.plist" >"$app/Contents/Info.plist"
    # Not notarized; an ad-hoc signature is still required to run on Apple silicon.
    codesign --force --deep --sign - "$app"
    ditto -c -k --norsrc --noextattr --noacl --keepParent "$app" "$dist/$name.zip"
    echo "$dist/$name.zip"
    ;;
  *-linux-*)
    name="vikui-$version-linux-$arch"
    mkdir -p "$stage/$name"
    cp "$release/vikui" "$root/README.md" "$stage/$name/"
    tar -C "$stage" -czf "$dist/$name.tar.gz" "$name"
    echo "$dist/$name.tar.gz"
    ;;
  *-windows-*)
    name="vikui-$version-windows-$arch"
    mkdir -p "$stage/$name"
    cp "$release/vikui.exe" "$root/README.md" "$stage/$name/"
    (cd "$stage" && 7z a -tzip -bso0 "$dist/$name.zip" "$name")
    echo "$dist/$name.zip"
    ;;
  *)
    echo "unsupported target: $target" >&2
    exit 1
    ;;
esac
