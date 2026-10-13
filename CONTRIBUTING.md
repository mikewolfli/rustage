# Contributing to rustage

[English overview](README.md) | [中文概览](README.zh-CN.md)

## Current stage

This repository contains design documents and contribution infrastructure,
plus a minimal Rust executable scaffold. It does not yet contain a runnable
designer. The scaffold pins its one third-party dependency, the
`rust-widgets` revision, and its Rust toolchain, and it carries runnable probes
that verify the dependency's layout, generation, event and unbind contracts.
Contributions may include design reviews, documentation fixes, reproducible
upstream API evidence, and roadmap refinement. Confirm the relevant
baseline decisions before adding GUI frameworks, dependencies, or new public
architecture.

## Before starting

1. Read the [principles](docs/plan/principle.md),
   [whitepaper](docs/plan/whitepaper.md), and [roadmap](docs/plan/roadmap.md).
   These design documents currently use Chinese.
2. Search existing issues and pull requests. Discuss substantial features or
   architectural changes through a feature or design issue first.
3. Identify the affected functional domain, task, or milestone.
   Roadmap task IDs are not automatically created GitHub issues.
4. Keep changes focused and preserve other contributors' work.

## Pull request workflow

1. Create a topic branch from the intended base branch.
2. Update directly related documentation. Keep both README versions consistent
   when status, scope, or user-facing instructions change.
3. Run the smallest relevant checks and record commands, environment,
   results, and anything not verified.
4. Open a pull request using the template, with scope, risks, and evidence.
5. Address review feedback. A submitted PR does not establish an approved decision.

Use clear commit subjects, optionally prefixed with `docs:`, `chore:`, `fix:`,
or `feat:`. You must have the right to contribute your changes under the
project's MIT license. Identify third-party sources and licenses.
Contributors retain their rights; no copyright assignment is required.

## Checks available today

Use Git, Rust, and Node.js 22 or newer. No npm packages are required; the one Rust dependency is
the pinned `rust-widgets` revision, which Cargo fetches over the network on a cold build. The Rust
toolchain is pinned by `rust-toolchain.toml`, so the checks run under the version the recorded
evidence was produced with:

```sh
cargo fmt --check
cargo test
cargo run -- --help
cargo run -- --probe-apis target/probe
node --test tools/check-docs.test.mjs
node tools/check-docs.mjs
node --test tools/resources/check-resources.test.mjs
node tools/resources/check-resources.mjs
node --test tools/support-matrix/check-support-matrix.test.mjs
node tools/support-matrix/check-support-matrix.mjs
node --test tools/decisions/check-decisions.test.mjs
node tools/decisions/check-decisions.mjs
node --test tools/contracts/check-contracts.test.mjs
node tools/contracts/check-contracts.mjs
node --test tools/journeys/check-journeys.test.mjs
node tools/journeys/check-journeys.mjs
node --test tools/fixtures/check-fixtures.test.mjs
node tools/fixtures/generate-fixtures.mjs --verify
node --test tools/benchmark/check-benchmark.test.mjs
node tools/benchmark/check-benchmark.mjs
git diff --check
```

Rust checks cover the command line, the icon seam and the icon strip: the semantic icon table
resolves in the pinned dependency, the host glyphs clear the dependency's contour budget and draw
at every displayed size, the eight colour roles stay legible and distinguishable in both
appearances, a theme switch reaches the controls, and the committed baselines under
`tests/baselines/icons/` still match what a real control renders. They also cover the dependency
contract probes (`--probe-apis`): layout geometry, mode 2 generation and its target-capability
query, signal dispatch order, connection teardown, and the text editor's edit primitives, guarded
`replace_range`, highlight state, graded scale, viewport coordinates and a cross-thread
highlighter, each tied to the pinned revision by a lockfile drift test. The command writes
`api-probes.json` under the directory you name and **exits non-zero while any probe reports a
gap** — a gap is a recorded finding, not a pass, and the open ones are listed in
[`docs/rw-feedback/`](docs/rw-feedback/feedback-20261013.md). These probes do not establish that a
window has displayed anything, that interaction states draw correctly, that a generated file
compiles for a given profile, or that a GUI designer or any output target is
implemented. Documentation checks
cover local Markdown file links, whitespace, conflict markers, required
community files, roadmap task dependencies, declared roadmap counts, and task
prerequisites across milestone assignments. They do not fetch external URLs,
validate heading anchors, or replace design review or end-to-end acceptance.

Host resource checks cover the icon manifest schema and semantic IDs, the pinned upstream token
contract, the glyph authoring subset (stroke centerlines, safe area, hole-free outlines, the
dependency's contour and point budgets), the theme role contract, licence provenance against the
redistributed copy, the branding masters and the packaging definitions of all four platforms.
They also rebuild `src/resources/generated_host_icons.rs` from the manifest and the SVG sources
and compare it byte for byte, so a hand-edited or stale compiled table fails the check. They do
not prove that a platform shows the packaged icon, or that the packaged identity is correct. Deriving the platform icons, the review
sheet and the theme matrix is a separate, optional step:

```sh
node tools/resources/generate-resources.mjs   # writes under target/resources/ and src/resources/
```

It needs an SVG rasteriser (`rsvg-convert`, `resvg` or macOS `sips`) and reports a blocked stage
when none is available instead of approximating the artwork. Both review sheets draw the expanded
outline — the geometry the renderer receives — rather than the authoring source, and both carry a
companion `.json` with the cell mapping.

The generator also rewrites the compiled icon table, which is the only file it writes inside the
source tree. Review that diff the way you review the manifest: it is derived, but it is what the
crate compiles. Regenerating the icon baselines is a separate command, and the baselines are
reviewed as pictures before they are replaced:

```sh
cargo run -- --render-icons tests/baselines/icons
cargo test   # fails, naming the file, while a baseline is stale
```

A new glyph is authored as `stroke` centerlines, never as a pre-flattened polygon, and never as a
second file for a theme or state: one source, `currentColor`, and the theme supplies the colour.

## Decision register

The conclusions behind the foundation tasks live in
[`docs/plan/decisions.json`](docs/plan/decisions.json), with a readable copy in
[`docs/plan/decisions.md`](docs/plan/decisions.md). The check compares the copy and whitepaper
§17.7 against the data, and refuses an entry that has no status, no rationale, no rejected
alternative, no compatibility impact, no revisit trigger, no owner or no affected task. An entry
with status `open` records a value only the project owner can supply; until that happens the
register is not the place to guess, and code must not depend on it.

## Shared contracts

The document contracts (`src/contracts/`) and the project's only writable model are described in
[`docs/plan/contracts.md`](docs/plan/contracts.md), with the machine-checked data in
[`docs/plan/contracts.json`](docs/plan/contracts.json). The check compares that document, the
whitepaper's agent-boundary table and the code: it refuses a mutating entry point that is not
declared, a public field on a declared file (which would bypass every entry point), and an invariant
that names a test which does not exist. If you add a write path, declare it and explain why it is
not a second writable model.

## Journey walkthrough

Every end-to-end journey in [roadmap section 34](docs/plan/roadmap.md) is walked step by step in
[`docs/plan/journeys.md`](docs/plan/journeys.md), with the data in
[`docs/plan/journeys.json`](docs/plan/journeys.json). The check compares the walkthrough with the
roadmap (names, evidence, milestones and the operation chain step by step), refuses an entry that is
not a declared menu group or panel, refuses a panel no mode shows, and refuses a step whose owner is
not a real roadmap task. A step without an entry must say why. The walkthrough records designs and
owners, not working menus: the data file's `implementation` stays `none` until a task wires one up.

## Fixtures and measurements

The benchmark data sets are described in [`fixtures/spec.json`](fixtures/spec.json) and produced on
demand, because the sizes T00.07 fixes are far too large to commit:

```sh
node tools/fixtures/generate-fixtures.mjs --output target/fixtures   # write every fixture
node tools/fixtures/generate-fixtures.mjs --verify                   # hashes only, proves reproducibility
```

The measurement contract is in [`docs/plan/benchmark.md`](docs/plan/benchmark.md), with the data in
[`docs/plan/benchmark.json`](docs/plan/benchmark.json). The check compares every scenario with
roadmap 35.2 - scenario text, threshold text and phase - and refuses a softened threshold, a
scenario whose data set does not advertise it, and a measurement claimed without an artifact,
revision and date. Nothing here is measured yet, and the document says so.

## Documentation and implementation expectations

- Community documents and GitHub templates use English primarily.
  The README is bilingual; design documents remain in Chinese.
- Distinguish proposed, planned, unverified, and verified capabilities.
- Link repository files relatively; link upstream material through its public
  repository rather than a sibling directory on your machine.
- Consider the model, UI, preview, generation, failure paths, permissions,
  and tests together.
- Do not remove failed tasks or quietly relax acceptance thresholds.
- Do not add placeholder success, empty implementations, or unused interfaces.
- Follow the project principles when changing source code.

## Security and community

Do not commit credentials, personal data, private projects, or unauthorized assets.
Follow [SECURITY.md](SECURITY.md) for vulnerabilities rather than publishing
details in a normal issue.
Participation follows [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).
See [SUPPORT.md](SUPPORT.md) for questions and response boundaries.
