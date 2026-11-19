#!/usr/bin/env bash
# Builds the Linux release artifacts from a `flutter build linux --release`
# bundle:
#   atropos-<v>-linux-x86_64.tar.gz       portable folder
#   atropos-<v>-linux-amd64.deb           Debian/Ubuntu package
#   pkgroot/                             staged tree reused by the Arch package
# Usage: build-packages.sh <version> <bundle dir> <out dir>
set -euo pipefail

version="$1"
bundle="$(realpath "$2")"
out="$(realpath -m "$3")"
here="$(cd "$(dirname "$0")" && pwd)"
app_id="io.github.aimatochysia.atropos"
icon="$here/$app_id.png"
mkdir -p "$out"

# Portable tarball.
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
portable="$work/atropos-$version-linux-x86_64"
cp -a "$bundle" "$portable"
cp "$here/$app_id.desktop" "$portable/"
cp "$icon" "$portable/$app_id.png"
tar -C "$work" -czf "$out/atropos-$version-linux-x86_64.tar.gz" "$(basename "$portable")"

# Shared install tree: /usr/lib/atropos + launcher, desktop entry and icon.
root="$out/pkgroot"
rm -rf "$root"
install -d "$root/usr/lib/atropos" "$root/usr/bin" \
  "$root/usr/share/applications" "$root/usr/share/icons/hicolor/512x512/apps"
cp -a "$bundle/." "$root/usr/lib/atropos/"
ln -s ../lib/atropos/atropos "$root/usr/bin/atropos"
install -m644 "$here/$app_id.desktop" "$root/usr/share/applications/"
install -m644 "$icon" "$root/usr/share/icons/hicolor/512x512/apps/$app_id.png"

# Debian package. Debian versions may not contain '-' in the upstream part
# without a revision, so pre-release dashes become '~' (sorts before final).
deb="$work/deb"
cp -a "$root" "$deb"
install -d "$deb/DEBIAN"
size="$(du -sk "$deb/usr" | cut -f1)"
cat > "$deb/DEBIAN/control" <<CONTROL
Package: atropos
Version: ${version//-/\~}
Section: devel
Priority: optional
Architecture: amd64
Installed-Size: $size
Depends: libgtk-3-0 | libgtk-3-0t64, libc6 (>= 2.35)
Maintainer: Atropos <noreply@github.com>
Homepage: https://github.com/aimatochysia/atropos
Description: Encrypt folders and files with hybrid post-quantum cryptography
 Atropos encrypts folders and files with ML-KEM-1024 + ECDH P-384 and
 AES-256-GCM so they can travel through cloud drives, USB sticks and git.
CONTROL
dpkg-deb --build --root-owner-group -Zxz "$deb" "$out/atropos-$version-linux-amd64.deb"
echo "Built Linux packages in $out"
