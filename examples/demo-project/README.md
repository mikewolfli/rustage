# Demo project

A project document in the version 1 format, kept in the repository so the format can be exercised
without inventing one first. It is not Rust code and no Rust code is built from this directory.

The format itself is documented where it is implemented:
[`src/project/record/format.rs`](../../src/project/record/format.rs).

Check it with:

```sh
cargo run -- --project-check examples/demo-project/project.json
```

The check reports what this build understands, what it keeps without reading, how each reference
resolved against the project root, and then writes `project.roundtrip.json` next to it and reads
that back, so "nothing was dropped" is something you can look at rather than something this file
claims. The round trip is ignored by Git because it is output.

Two things are deliberately missing from the directory:

* `pages/main.json` — a page's own document. Its schema arrives with task T02.02 (the empty project
  and the basic page template); until then the reference resolves inside the root and is reported as
  *not there yet*, which is what a project being built up looks like.
* `src/generated/main.rs` — a generated file. Nothing generates it yet, and this build only writes
  generated code into `src/generated/` (decision `F2`).
