#!/bin/sh
# SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
# SPDX-License-Identifier: MIT
#
# issue-20261013 §2.2: the end-to-end check with a fixture identity.
#
#   1. build the pinned rust-widgets for aarch64-unknown-linux-ohos (through bridge-check.sh);
#   2. write a throwaway identity, generate the complete package tree from the declared
#      templates, and assert every HarmonyOS file is there with no placeholder left;
#   3. put the built library where the scaffold expects it and run `hvigorw assembleHap`;
#   4. assert the unsigned HAP contains the ArkTS module, libentry.so and librust_widgets.so.
#
# The identity is a FIXTURE. It is written under a temporary directory and never into the
# repository; T00.04 still owns the real application id, minimum OS levels and signing. Signing
# and a 2in1 device are therefore explicitly out of scope here.
#
# Environment (all optional):
#   OHOS_SDK_NATIVE  the SDK's native/ directory
#   RUST_WIDGETS_DIR the pinned rust-widgets checkout
#   DEVECO_HOME      DevEco Studio's Contents directory (for hvigorw)
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
sdk=${OHOS_SDK_NATIVE:-"$HOME/Library/OpenHarmony/Sdk/20/native"}
rust_widgets=${RUST_WIDGETS_DIR:-"$root/../rust-widgets"}
deveco=${DEVECO_HOME:-/Applications/DevEco-Studio.app/Contents}
target=aarch64-unknown-linux-ohos

fail() {
    echo "hap-check: $1" >&2
    exit 1
}

command -v node >/dev/null 2>&1 || fail "node is required (the resource generator is a Node script)"
command -v unzip >/dev/null 2>&1 || fail "unzip is required to inspect the produced HAP"
[ -x "$sdk/llvm/bin/aarch64-unknown-linux-ohos-clang" ] \
    || fail "$sdk/llvm/bin/aarch64-unknown-linux-ohos-clang is missing; set OHOS_SDK_NATIVE"
[ -f "$rust_widgets/include/rw_generated.h" ] \
    || fail "$rust_widgets/include/rw_generated.h is missing; set RUST_WIDGETS_DIR"

hvigorw=${HVIGORW:-"$deveco/tools/hvigor/bin/hvigorw"}
[ -x "$hvigorw" ] || hvigorw=$(command -v hvigorw || true)
[ -n "$hvigorw" ] && [ -x "$hvigorw" ] || fail "hvigorw not found; set HVIGORW or DEVECO_HOME"

work=$(mktemp -d "${TMPDIR:-/tmp}/rustage-hap.XXXXXX")
trap 'rm -rf "$work"' EXIT INT TERM

echo "== checking the native bridge first"
sh "$root/tools/harmony/bridge-check.sh" >/dev/null 2>&1 \
    || fail "the native bridge check failed; run tools/harmony/bridge-check.sh for the reason"

echo "== writing a fixture identity (not a product decision)"
SOURCE_IDENTITY="$root/packaging/identity.json" IDENTITY_PATH="$work/identity.json" \
    node -e '
      const fs = require("fs");
      const identity = JSON.parse(fs.readFileSync(process.env.SOURCE_IDENTITY, "utf8"));
      identity.status = "decided";
      identity.application_id = "com.example.rustagetest";
      identity.install_prefix = "/opt/rustage";
      identity.minimum_os = {
        windows: "10.0.19041", macos: "13.0", linux: "glibc 2.31", harmonyos: "6.0.0(20)",
      };
      identity.distribution = {
        windows: "msi", macos: "dmg", linux: "deb", harmonyos: "appgallery",
      };
      fs.writeFileSync(process.env.IDENTITY_PATH, `${JSON.stringify(identity, null, 2)}\n`);
    ' || fail "could not write the fixture identity"

echo "== generating the package tree from the declared templates"
generated="$work/generated"
node "$root/tools/resources/generate-resources.mjs" --out "$generated" --identity "$work/identity.json" >"$work/generate.log" \
    || { tail -20 "$work/generate.log"; fail "resource generation failed"; }
grep -q "ok.*package-metadata" "$work/generate.log" || {
    tail -20 "$work/generate.log"
    fail "package-metadata did not complete; the identity fixture is incomplete"
}
project="$generated/package/harmonyos"
[ -f "$project/entry/src/main/module.json5" ] || fail "the generated tree has no module manifest"
[ -f "$project/build-profile.json5" ] || fail "the generated tree has no build profile"
if grep -rq "@@" "$project" --include="*.json5" --include="*.json" --include="*.ets" --include="*.cpp"; then
    fail "the generated tree still carries a placeholder"
fi
grep -Eq '"versionCode": [0-9]+,' "$project/AppScope/app.json5" \
    || fail "the generated versionCode is not a JSON number"

echo "== placing the built library and building the HAP"
cp "$rust_widgets/target/$target/debug/librust_widgets.so" \
    "$project/entry/libs/arm64-v8a/librust_widgets.so"
(
    cd "$project"
    DEVECO_SDK_HOME="${DEVECO_SDK_HOME:-$deveco/sdk}" \
    RUST_WIDGETS_INCLUDE="$rust_widgets/include" \
        "$hvigorw" assembleHap --no-daemon >"$work/hvigor.log" 2>&1
) || { tail -30 "$work/hvigor.log"; fail "hvigor assembleHap failed"; }

hap="$project/entry/build/default/outputs/default/entry-default-unsigned.hap"
[ -f "$hap" ] || fail "no unsigned HAP at $hap"
for entry in "libs/arm64-v8a/libentry.so" "libs/arm64-v8a/librust_widgets.so" "ets/modules.abc"; do
    unzip -l "$hap" | grep -q "$entry" || fail "the HAP is missing $entry"
done

echo "hap-check: ok (unsigned HAP with libentry.so + librust_widgets.so)"
echo "not covered: signing, installation and a 2in1 device run (T00.04 identity + signing)"
