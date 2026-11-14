#!/usr/bin/env bash
# 构建 Android 端 EasyTier 原生库。
#
# 产物：
#   android/app/src/main/jniLibs/<abi>/libeasytier_android_jni.so
#   android/app/src/main/jniLibs/<abi>/libeasytier_ffi.so
#   android/app/src/main/jniLibs/<abi>/libtun_easytier-*.so 等依赖
#
# 依赖：cargo-ndk、Android NDK、patchelf、curl、tar。
# 首次构建会下载 EasyTier 源码和 Rust 依赖，耗时较长。
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VERSION="${EASYTIER_VERSION:-2.6.4}"
SRC="$ROOT/target/easytier-src"
OUT="$ROOT/android/app/src/main/jniLibs"
CARGO_HOME="${CARGO_HOME:-$ROOT/.cargo}"
CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target/easytier-build}"
export CARGO_HOME CARGO_TARGET_DIR
export PATH="$CARGO_HOME/bin:$PATH"
export RUSTUP_TOOLCHAIN="${RUSTUP_TOOLCHAIN:-stable}"

: "${ANDROID_NDK_HOME:?请设置 ANDROID_NDK_HOME}"
export ANDROID_NDK_ROOT="$ANDROID_NDK_HOME"

if ! command -v cargo-ndk >/dev/null 2>&1; then
  echo "缺少 cargo-ndk，请先 cargo install cargo-ndk" >&2
  exit 1
fi
if ! command -v patchelf >/dev/null 2>&1; then
  echo "缺少 patchelf，请先安装" >&2
  exit 1
fi

if [ ! -d "$SRC" ]; then
  echo "下载 EasyTier v$VERSION 源码…"
  TARBALL="$ROOT/target/easytier-v$VERSION.tar.gz"
  if [ ! -f "$TARBALL" ]; then
    curl -fL --retry 3 -o "$TARBALL" \
      "https://github.com/EasyTier/EasyTier/archive/refs/tags/v$VERSION.tar.gz"
  fi
  mkdir -p "$SRC"
  tar -xzf "$TARBALL" -C "$SRC" --strip-components=1
fi

mkdir -p "$OUT"

build_one() {
  local abi="$1"
  echo "==> $abi"
  (cd "$SRC" && cargo ndk -t "$abi" -P 26 -o "$OUT" build --release -p easytier-android-jni)
  (cd "$SRC" && cargo ndk -t "$abi" -P 26 -o "$OUT" build --release -p easytier-ffi)

  local jni="$OUT/$abi/libeasytier_android_jni.so"
  if [ ! -f "$jni" ]; then
    echo "找不到 $jni" >&2
    exit 1
  fi
  if ! readelf -d "$jni" | grep -q 'libeasytier_ffi.so'; then
    patchelf --add-needed libeasytier_ffi.so "$jni"
  fi
}

build_one arm64-v8a
build_one x86_64

echo "完成：$OUT"
