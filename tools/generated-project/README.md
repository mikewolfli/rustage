# Generated project (`tools/generated-project`)

The MS07 exit demonstration as a repeatable harness: generate a page with the real binary, write a
standalone consumer crate that depends on the same pinned `rust_widgets`, build it, and run it.

```text
cargo build --bin rustage
python3 tools/generated-project/run.py
```

Expected tail:

```text
precheck: clean
report: ...
wrote .../src/ui.rs (... bytes)
generated-ui-ok
generated project: ... bytes of source built and ran
generated project binary: target/debug/generated-consumer
```

It is not part of `cargo test`: the consumer rebuilds the widget crate, and doing that inside a test
run would be slow and would fight the test's build lock. `tests/generate_write.rs` covers the cheap
half (the write point and blocked-precheck behaviour) in the normal test suite.

What it does **not** prove: that mini/embedded source links (their evidence is the profile rlib), or
that a window is visible on screen (that is the host/device layer).
