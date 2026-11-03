# The prebuilt Rust library (`entry/libs/arm64-v8a`)

This directory is where `librust_widgets.so` goes. It is not committed: the file is a
build product of the pinned revision, not a source file, and a checked-in binary would
let a stale build masquerade as the current contract.

Build it on a machine with the OpenHarmony SDK and a Rust toolchain that has the
`aarch64-unknown-linux-ohos` target:

```sh
rustup target add aarch64-unknown-linux-ohos

OHOS_SDK_NATIVE=<sdk>/native \
CARGO_TARGET_AARCH64_UNKNOWN_LINUX_OHOS_LINKER=<sdk>/native/llvm/bin/aarch64-unknown-linux-ohos-clang \
cargo build --manifest-path <rust-widgets>/Cargo.toml \
  --no-default-features \
  --features "desktop xcomponent runtime-fonts" \
  --lib --target aarch64-unknown-linux-ohos

cp <rust-widgets>/target/aarch64-unknown-linux-ohos/debug/librust_widgets.so \
  entry/libs/arm64-v8a/
```

The `xcomponent` feature is required: without it `rw_harmony_bind_xcomponent` compiles
to a `false` that logs "no XComponent bridge", and the page's `onLoad` would fail
honestly instead of rendering.

A device profile (`desktop` here) is equally required: the C ABI module is gated on
`any(desktop, jni, mobile-api)`, so a build with only `harmony …` exports the
Harmony-specific callbacks but **not** `rw_init`/`rw_create_*`/`rw_mount_surface`.
Such a library links against this module only because a shared object may leave
symbols undefined; the HAP then fails when ArkTS loads it. With `desktop`, the
library exports 165 `rw_*` symbols and this module's 13 undefined `rw_*` symbols
all resolve.

The `debug` library is ~400 MB and is only for a spike; a release build for a packaged
HAP is a separate task (`cargo build --release`, and the packaging decision in
`packaging/harmonyos/app.json` `distribution`).
