#!/usr/bin/env bash
# 生成 Arch / Debian / RPM 安装包。
# 需要：makepkg、fakeroot、ar、gzip、tar；RPM 需要 rpmbuild（系统或 .tools/rpmroot）。
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
BIN="${BIN:-$ROOT/dist/asa-${VERSION}-linux-x86_64}"
if [ ! -x "$BIN" ]; then
  BIN="$ROOT/dist/asa"
fi
[ -x "$BIN" ] || { echo "找不到 Linux 二进制：$BIN" >&2; exit 1; }

OUT="$ROOT/dist/packages"
BUILD="$ROOT/packaging/build"
rm -rf "$BUILD"
mkdir -p "$OUT" "$BUILD"

COMMON="$BUILD/common"
mkdir -p "$COMMON"
cp "$BIN" "$COMMON/asa-$VERSION"
cp packaging/asa.desktop packaging/asa.1.gz packaging/asa.png LICENSE README.md "$COMMON"

echo "==> Arch"
ARCH="$BUILD/arch"
mkdir -p "$ARCH"
cp packaging/arch/PKGBUILD "$ARCH/"
cp "$COMMON"/* "$ARCH/"
(
  cd "$ARCH"
  PKGDEST="$OUT" makepkg -f --noconfirm --nodeps
)
rm -rf "$ARCH/src" "$ARCH/pkg"

echo "==> Debian"
DEB="$BUILD/deb"
install -Dm755 "$BIN" "$DEB/usr/bin/asa"
install -Dm644 packaging/asa.desktop "$DEB/usr/share/applications/asa.desktop"
install -Dm644 packaging/asa.1.gz "$DEB/usr/share/man/man1/asa.1.gz"
install -Dm644 packaging/asa.png "$DEB/usr/share/icons/hicolor/256x256/apps/asa.png"
install -Dm644 LICENSE "$DEB/usr/share/doc/asa/copyright"
install -Dm644 README.md "$DEB/usr/share/doc/asa/README.md"
mkdir -p "$DEB/DEBIAN"
SIZE="$(du -sk "$DEB/usr" | cut -f1)"
cat > "$DEB/DEBIAN/control" <<EOF
Package: asa
Version: $VERSION
Architecture: amd64
Maintainer: Kalore-N <zhanghaoyu2421@outlook.com>
Section: net
Priority: optional
Installed-Size: $SIZE
Description: 去中心化 TUI 群聊
 ASA is a decentralized terminal chat client with end-to-end
 encryption, file transfer, fixed room entries and an optional relay.
EOF
(
  cd "$DEB"
  find usr -type f -print0 | sort -z | xargs -0 md5sum > DEBIAN/md5sums
  tar --owner=0 --group=0 --numeric-owner -czf "$BUILD/data.tar.gz" usr
)
(
  cd "$DEB/DEBIAN"
  tar --owner=0 --group=0 --numeric-owner -czf "$BUILD/control.tar.gz" control md5sums
)
printf '2.0\n' > "$BUILD/debian-binary"
(
  cd "$BUILD"
  ar rcs "$OUT/asa_${VERSION}_amd64.deb" debian-binary control.tar.gz data.tar.gz
)

echo "==> RPM"
RPMBUILD="${RPMBUILD:-}"
if [ -z "$RPMBUILD" ]; then
  if command -v rpmbuild >/dev/null 2>&1; then
    RPMBUILD="rpmbuild"
  elif [ -x "$ROOT/.tools/rpmroot/usr/bin/rpmbuild" ]; then
    RPMBUILD="$ROOT/.tools/rpmroot/usr/bin/rpmbuild"
    export LD_LIBRARY_PATH="$ROOT/.tools/rpmroot/usr/lib:${LD_LIBRARY_PATH:-}"
    export RPM_CONFIGDIR="$ROOT/.tools/rpmroot/usr/lib/rpm"
  else
    echo "缺少 rpmbuild；可安装 rpm-tools，或用 .tools/rpmroot 便携解包。" >&2
    exit 1
  fi
fi
RPMTOP="$BUILD/rpm"
mkdir -p "$RPMTOP/SOURCES" "$RPMTOP/SPECS" "$RPMTOP/BUILD" "$RPMTOP/RPMS" "$RPMTOP/SRPMS"
cp "$COMMON"/* "$RPMTOP/SOURCES/"
cp packaging/rpm/asa.spec "$RPMTOP/SPECS/"
"$RPMBUILD" --define "_topdir $RPMTOP" -bb "$RPMTOP/SPECS/asa.spec"
find "$RPMTOP/RPMS" -name '*.rpm' -exec cp -v {} "$OUT/" \;

echo
echo "产物："
ls -lh "$OUT"
