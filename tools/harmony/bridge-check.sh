#!/bin/sh
# SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
# SPDX-License-Identifier: MIT
#
# H-28's evidence as one repeatable check:
#
#   1. build the pinned rust-widgets for aarch64-unknown-linux-ohos with a device
#      profile and the xcomponent feature;
#   2. assert the library exports the C entry points the scaffold calls;
#   3. configure and build the scaffold's CMake project with the OpenHarmony
#      toolchain, so `--no-undefined` proves every `rw_*` call resolves;
#   4. assert the module entry point and its NEEDED libraries.
#
# It does NOT build a HAP or run on a device: both need the application identity,
# the minimum OS level and signing material, which T00.04 still leaves open.
# A failure above says the scaffold or the library is wrong; it says nothing
# about ArkUI, hvigor or a 2in1 device.
#
# Usage:
#   sh tools/harmony/bridge-check.sh
#
# Environment (all optional):
#   OHOS_SDK_NATIVE  the SDK's native/ directory (default: the macOS install)
#   RUST_WIDGETS_DIR the pinned rust-widgets checkout (default: ../rust-widgets)
#   OHOS_CMAKE       cmake executable (default: DevEco's, then PATH)
#   OHOS_NINJA       ninja executable (default: DevEco's, then PATH)
#
# Exit codes: 0 the bridge builds and links; 1 any step refused.
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
sdk=${OHOS_SDK_NATIVE:-"$HOME/Library/OpenHarmony/Sdk/20/native"}
rust_widgets=${RUST_WIDGETS_DIR:-"$root/../rust-widgets"}
deveco=${DEVECO_HOME:-/Applications/DevEco-Studio.app/Contents}
target=aarch64-unknown-linux-ohos
features="desktop xcomponent runtime-fonts"

fail() {
    echo "bridge-check: $1" >&2
    exit 1
}

[ -f "$rust_widgets/include/rw_generated.h" ] \
    || fail "$rust_widgets/include/rw_generated.h is missing; set RUST_WIDGETS_DIR to the pinned checkout"
[ -x "$sdk/llvm/bin/aarch64-unknown-linux-ohos-clang" ] \
    || fail "$sdk/llvm/bin/aarch64-unknown-linux-ohos-clang is missing; set OHOS_SDK_NATIVE"
rustup target list --installed | grep -qx "$target" \
    || fail "rustup target $target is not installed (rustup target add $target)"

cmake=${OHOS_CMAKE:-"$deveco/sdk/default/openharmony/native/build-tools/cmake/bin/cmake"}
[ -x "$cmake" ] || cmake=$(command -v cmake || true)
[ -n "$cmake" ] && [ -x "$cmake" ] || fail "cmake not found; set OHOS_CMAKE"

ninja=${OHOS_NINJA:-"$deveco/sdk/default/openharmony/native/build-tools/cmake/bin/ninja"}
[ -x "$ninja" ] || ninja=$(command -v ninja || true)
[ -n "$ninja" ] && [ -x "$ninja" ] || fail "ninja not found; set OHOS_NINJA"

toolchain="$sdk/build/cmake/ohos.toolchain.cmake"
[ -f "$toolchain" ] || fail "$toolchain is missing; is OHOS_SDK_NATIVE the native/ directory?"

work=$(mktemp -d "${TMPDIR:-/tmp}/rustage-bridge.XXXXXX")
trap 'rm -rf "$work"' EXIT INT TERM

echo "== building rust_widgets ($target, features: $features)"
OHOS_SDK_NATIVE="$sdk" \
CC_aarch64_unknown_linux_ohos="$sdk/llvm/bin/aarch64-unknown-linux-ohos-clang" \
AR_aarch64_unknown_linux_ohos="$sdk/llvm/bin/llvm-ar" \
CARGO_TARGET_AARCH64_UNKNOWN_LINUX_OHOS_LINKER="$sdk/llvm/bin/aarch64-unknown-linux-ohos-clang" \
    cargo build --manifest-path "$rust_widgets/Cargo.toml" \
        --no-default-features --features "$features" \
        --lib --target "$target" >/dev/null \
    || fail "the library did not build for $target"

library="$rust_widgets/target/$target/debug/librust_widgets.so"
[ -f "$library" ] || fail "$library was not produced"

echo "== checking the library's C ABI exports"
for symbol in rw_init rw_create_window rw_create_button rw_mount_surface \
    rw_report_window_resize rw_harmony_bind_xcomponent rw_harmony_bind_node; do
    "$sdk/llvm/bin/llvm-nm" -D "$library" | grep -q " T $symbol\$" \
        || fail "$symbol is not exported; build with a device profile (desktop/jni/mobile-api), not only 'harmony'"
done

echo "== configuring and building the NAPI module (CMake, --no-undefined)"
"$cmake" -S "$root/packaging/harmonyos/entry/src/main/cpp" -B "$work" -G Ninja \
    -DCMAKE_MAKE_PROGRAM="$ninja" \
    -DCMAKE_TOOLCHAIN_FILE="$toolchain" \
    -DOHOS_ARCH=arm64-v8a \
    -DRUST_WIDGETS_INCLUDE="$rust_widgets/include" \
    -DRUST_WIDGETS_LIB_DIR="$rust_widgets/target/$target/debug" \
    -DCMAKE_BUILD_TYPE=Debug >/dev/null \
    || fail "CMake configuration failed"
"$cmake" --build "$work" >/dev/null || fail "the NAPI module did not build"

module="$work/libentry.so"
[ -f "$module" ] || fail "$module was not produced"
"$sdk/llvm/bin/llvm-readelf" -d "$module" | grep -q "librust_widgets.so" \
    || fail "libentry.so does not depend on librust_widgets.so"
"$sdk/llvm/bin/llvm-nm" -D "$module" | grep -q " T EntryModuleInit\$" \
    || fail "libentry.so does not export EntryModuleInit"

echo "bridge-check: ok ($(basename "$module"))"
echo "this script does not cover: ArkTS compilation, hvigor assembleHap, 2in1 device run"
echo "(a one-off unsigned HAP build with a temporary identity is recorded in docs/log/log-20261103-2.md section 8)"
