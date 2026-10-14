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

Use Git, Rust, and Node.js 22 or newer. No npm packages are required. The Rust dependencies are the
pinned `rust-widgets` revision, which Cargo fetches over the network on a cold build, and `libc` on
Unix only (for the process-group signals T01.04 needs; its resolution is already in `Cargo.lock`
through the widget dependency). The Rust toolchain is pinned by `rust-toolchain.toml`, so the checks
run under the version the recorded evidence was produced with:

```sh
cargo fmt --check
cargo test
cargo run -- --help
cargo run -- --probe-apis target/probe
cargo run -- --probe-host target/host        # host platform services; exits 4 while a check is not ready
cargo run -- --project-check examples/demo-project/project.json   # project format; exits 4 when it refuses to write
cargo run -- --new-project target/new-project --template application   # creates a project; exits 4 when refused
cargo run -- --open-project target/new-project/project.json   # opens it read-only and prints what would be asked
cargo run -- --recent                                        # projects opened before, and where they are now
cargo run -- --project-tree target/new-project/project.json  # what the project has, classified
cargo run -- --drop-entry target/new-project/project.json --document pages/main.json --trust
cargo run -- --file-op target/new-project/project.json --move pages/main.json pages/windows/main.json   # preview; writes nothing
cargo run -- --file-op target/new-project/project.json --move pages/main.json pages/windows/main.json --apply --trust
cargo run -- --configuration target/new-project/project.json          # which target, what each value came from
cargo run -- --configuration target/new-project/project.json --target default --remember   # choose one for this machine
cargo run -- --import target/new-project/project.json          # what it is, and what an import would change
cargo run -- --import target/new-project/project.json --check-backup   # what a past migration kept
cargo run -- --validate target/new-project/project.json      # what it needs, and whether it is there; writes nothing
cargo run -- --validate target/new-project/project.json --diagnose --trust   # include the toolchain (runs two programs)
cargo run -- --validate target/new-project/project.json --record target/scan.json   # keep a scan to compare against
cargo run -- --validate target/new-project/project.json --rescan target/scan.json   # what changed since that scan
cargo run -- --new-workspace target/area --name "Area"   # one document, no members yet
cargo run -- --workspace target/area/workspace.json      # the members, what each holds, and the files qualified by member
cargo run -- --workspace target/area/workspace.json --add app=app --apply --trust   # names a root that is already there
cargo run -- --workspace target/area/workspace.json --remove lib --apply --trust    # the directory stays where it is
cargo run -- --workspace target/area/workspace.json --file lib:pages/main.json      # which of two same-named files this is
cargo run -- --workspace target/area/workspace.json --activate lib                  # which member is active, on this machine
cargo run -- --launch --exit-after-ms 600    # baseline desktop session, bounded
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
node --test tools/traceability/check-traceability.test.mjs
node tools/traceability/check-traceability.mjs
node --test tools/capability/check-capability.test.mjs
node tools/capability/check-capability.mjs
node --test tools/host-probe/check-host-probe.test.mjs
node tools/host-probe/check-host-probe.mjs
git diff --check
```

`--launch` opens the baseline desktop session and leaves it again. It refuses to report a session
when the dependency reports a non-interactive backend, and it says in the log how the session ended
(`LoopEnd::WindowClosed` for a window close, `LoopEnd::HostEnded` when the host had to end its own
process because the pinned revision's `quit()` only works on the thread that owns the GUI toolkit).
On a machine without a desktop session it exits with code 3 and a reason; that is a refusal, not a
failure, and not a success. Rust checks cover the command line, the icon seam and the icon strip: the semantic icon table
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
[`docs/rw-feedback/`](docs/rw-feedback/feedback-20261013.md).
`--probe-host` checks the host's own services — whether a baseline session starts, whether pointer and
keyboard input reaches a mounted control, the operating system's file chooser, the painted
chooser frame, the clipboard, the host directories, the URL handler, directory watching and the
background executor (whose check proves that a submission returns while the job runs, that `cancelling`
is observable, and that a job ignoring its deadline is reported as abandoned), process ownership
(whose check starts three processes — one of this session, one started by that one, and one started
outside the session entirely — and requires the outsider to survive), and project trust (whose check
requires an untrusted project to be refused *and* to write nothing, then trusts the project,
authorizes the execution scope, runs the command and sees its marker file, then withdraws the
authorization and requires the refusal back — printing the authorization record it produced), and the
notification centre (whose check feeds it a really failed task, a really failing process, a program that
does not exist and a real permission refusal, and requires each to arrive with a cause, a next step and a
link, with the repeated failure counted rather than raised again), and the project registry (whose check
opens two real projects, starts a real process in each, closes one, and requires that project's process to
be gone, the other's to survive, the closed handle to refuse new work and the other project to keep its own
trust) — and writes `host-readiness.json` with the evidence for each, ending **not ready when a capability
is missing or when the environment cannot produce the evidence**. It exits 4 in that case, which is
`ExitCode::Reported` — "a real finding was reported" rather than a defect in the host. These probes do not establish that a
window has displayed anything, that interaction states draw correctly, that a generated file
compiles for a given profile, or that a GUI designer or any output target is
implemented. The per-host probe record ([`docs/plan/host-probe.md`](docs/plan/host-probe.md))
is checked against [`docs/plan/host-probe.json`](docs/plan/host-probe.json) and against the
probe's own source: the check names are read out of `Capability::name` in
`src/host/readiness.rs` and have to be covered exactly once, a cell may only claim `probed`
on the host the record was produced on, and a blocked host has to say what is missing. It
cannot run a macOS or Windows machine, so it proves the record is complete and consistent —
not that its claims are true.
`--project-check` reads a project document and reports what this build makes of it: the format
version and what will happen to the file, the identity, the counts, every member it keeps without
reading (decision `F1` requires them preserved and reported, never dropped), and every path in the
document with how it resolved against the project root. It then writes the document back through
this build's own writer, reads that file again, and compares: a document whose unknown members do not
come back has failed the check. A version this build does not know is opened read-only, only its
identity is read, and the writer is not reachable at all — the round trip is refused and `Reported`
(4) is the exit status, because "a document I must not write" is a real finding rather than a defect.
The demo under [`examples/demo-project/`](examples/demo-project/README.md) is a committed version 1
document that a Rust test pins to exactly what the writer writes, so the format cannot drift away
from its example. This does not establish that a page document can be written yet (its schema arrives
with T02.02) or that a project can be created, opened or migrated by the designer.

`--new-project <directory>` creates a project: it plans first (which is what the wizard's last step
shows: every file, and what could still go wrong), writes the files into a staging directory *beside*
the project, and puts the project in place with a single rename. That is where the acceptance line
"a failure must not overwrite the directory that was there and must not leave half a project" is
enforced: a directory that already holds anything is refused before anything is written, and every
refusal and every cancel removes the staged copy and reports what it removed. The staged project is
read back by the same readers the designer uses before it is put in place, so a created project is
one this build can open, not one it hopes it can. `--template empty|basic-page|application` chooses
what the project starts from and `--name` sets the name a person sees. This does not establish that a
window opened, that the templates are what a person wants, or that the designer's own menus create
projects — the wizard's UI arrives with the shell (T04/T26).

`--open-project <file>` reads a project and decides nothing: it prints the format verdict, the
identity, whether this build may write it, and the question a person would be asked about the
directory (what is in it that could run code during a build, what answering yes allows, what
answering no costs). Trust is one decision away and never implied: `--trust` answers the question,
`--diagnose` reads the toolchain — and reading a toolchain *runs two programs*, so it goes through the
permission gate and an untrusted project gets "not checked: reading a toolchain runs programs"
instead of a version nobody authorized. The question is re-read when it is answered, so a directory
that gained a `build.rs` while the prompt was on screen is refused and trust is withdrawn rather than
applied to a state the person never saw. Opening also records the project in a recent list under the
host's configuration directory — a place, never a permission: the list holds paths, names, identities
and times, it has no permission service and no way to start a program, and `--recent` prints it,
notices which entries are gone and remembers that. Repairing a moved entry is by identity: a path
that now holds a different project is refused with both identities named. This does not establish
that a window, a toolbar or a dialog exists, that a toolchain can build this project, or that the
wizard's steps have buttons — those are the shell's and the build tasks'.

`--project-tree <file>` shows what a project has, grouped the way a project tree groups it — pages
(which are also the entries), resources, generated files, stylesheets, target profiles and network
services — with, per item, whether the file behind it is present, missing or refused. `--drop-entry
<file> --document <path>` removes an entry and is where this milestone's acceptance lives: an entry
that other things still point at (a generated file that comes from it, the target's startup entry) is
**not** removed quietly. Without `--fix` the removal is refused, listing every reference, and nothing
is written at all; with `--fix` the references are rewritten and every one of them is reported, the
result is checked with the reader's own rules, and the document is put in place through the same write
path a save uses (conflict check, migration backup, atomic write) after the project is trusted and the
`write-file` permission is granted. `--project-tree` and this removal are the model and the command
line; the tree panel, its icons and its drag-and-drop belong to the shell (T04/T09).

`--file-op <file>` is where this milestone's second acceptance line lives: adding, excluding, moving,
renaming and deleting a file. The operation is planned first — `FilePlan::plan` reads the project and
writes nothing, and its plan says what would move, which references would be rewritten and, when the
operation cannot be done, why: a file a page's design still names, a page's own document, or a member
that a delete would leave naming a file that is gone, is **blocked** with the references listed, never
quietly dropped. Without `--apply` that is all that happens: the preview asks no question and writes
nothing, which is what the design surface shows before anyone agrees. With `--apply` the project is
trusted and the `write-file` permission is taken, the files move first and the document is written
last, so a failure lands on the write: the moves are then undone in reverse order and the command
reports what was put back and what was not, and a failed operation exits 4 even when everything was
recovered — a failure is never reported as a success. Excluding and deleting are kept apart, which is
what the roadmap asks for: excluding takes a declaration away and leaves the file, while deleting
takes the file off the disk as well, and it does so by holding the file inside the project
(`.rustage-deleted/<its own path>`) until the document that named it has been written — a failed write
puts it back, and if the held copy cannot be removed afterwards the command says where it is instead
of reporting a clean delete. A file's path, the references to it and the operation's own failures
therefore all have a stated outcome; rewriting the *design* files a resource is used in, undo of a
successful operation, and the file operations' own window belong to later tasks (T03/T05).

`--configuration <file>` is where this milestone's third acceptance line lives: "the source is
visible, and an active configuration does not leak between projects". A value is never just a value
— `Setting` carries the layer it came from and the values it replaced, so the report reads "profile:
release (from target `release`; it replaced `debug` from the built-in default)", and a layer that
says nothing is not a layer that said "no": silence means the layer below decides. The layers are the
built-in default, the target the project document declares, and this machine's overrides; there is
deliberately no project-level profile member, because version 2 of the format does not have one and
adding one would be a format change this task does not need. The active target and the overrides are
machine-local state (the host's configuration directory), and every entry in that file carries the
**identity** of the project it was recorded for: what was chosen for one project is reported as
belonging to another rather than used, an override recorded for another project or another target is
listed under "not applied" with the reason, and a resolution carries the identity it was made for —
`ResolvedConfiguration::for_project` is what refuses it to anybody else. `--remember` records the
target and the overrides for this project; `--forget` removes them. The credential side of the same
view lists each service's credential as a reference — the name, where the material lives (`system
credential store`, or an environment variable) and the health the host's store reports — and never
the material, which is why the view has no field a secret could be put in. A settings window, the
per-setting editors and the credential picker belong to the shell (T26); kit-versus-profile and the
target triple belong to T02.10.

`--import <file-or-directory>` is where this milestone's fourth acceptance line lives: "a diff before
and after the import, and a failed migration keeps the original project". Reading decides nothing and
writes nothing; `Import::candidate` is the plan, and it is two things a person looks at before
agreeing: the versioned steps that would run, and the diff between the document as it was read and
the document as this build would write it. The diff is by member, not by line, and members that do
not change are left out — the report is about what the import would do. `--apply` is the only path
that writes, so it is the only one that asks the project's question and takes the write permission;
it then works in this order: the file on disk must still be the file that was read (an edit made
while the plan was on screen stops the import, and the edit is the file that stays); a backup that is
already there stops it (a second migration must not overwrite the record of the first); the backup is
written and read back **byte for byte**; the migration is written; the result is read back and must be
the version this build writes. A failure before the write leaves the project exactly as it was, and a
failure after it puts the original bytes back from the backup — only a restore that also failed is
reported as something left behind. `--check-backup` reports the files a past migration kept: what
version they are, how many bytes, and whether this build reads them back as a document. Importing
another tool's format, merging two projects and migrating downwards all belong elsewhere (T02.10/T28).

`--validate <file>` is where this milestone's fifth acceptance line lives: "a missing SDK or resource
does not clear the editable document". Checking is answering, and the types say so: there is no writer
in `src/project/validate.rs` at all, and the command prints, as its last line, the project's own bytes
compared with the ones it read when it started. Every need is answered with one of four words, because
folding them together is how a report starts lying: `present`, `missing`, `not-checked` (nobody looked
— the project is not trusted, the permission table said no, the store could not be reached) and
`refused` (the path leaves the project root, so it was never resolved). An unread toolchain is
therefore *not* a missing SDK: `--diagnose --trust` reads it, and only then does a program that did not
answer become a missing one. Needs are identified the way a scan identifies an item — by the path the
document writes — so a file that three members name is one need and one item, and the two reports of
the same document agree about how much there is. Each need also carries where its answer came from,
and the sources fold into one line: the project's own files, this machine, this machine's credential
store. Nothing in a check asks a far end, and the mapping from a kind of need to a source is a total
function with no arm that is the network — which is what "this project opens offline" means here, and
it was run once inside a network namespace (`unshare -rn`, where `curl` cannot reach a host) rather
than asserted in prose. Opening the project still reads nothing off the disk it should not: validation
never writes, and the only path that writes anything at all is `--record`, which writes a scan to the
file the caller names and refuses to write it over the project document *or over any file the project
declares* — a report that destroys what it is about is not a report. A refusal in the
command (a record that is the project, a baseline that belongs to another project or a newer build, a
document this build does not interpret) exits 4 while still printing that the file was not touched:
"nothing was written" describes the file, and reporting it as the command's own success is how a
refusal comes out looking like one. Reusing the SDK/kit identities T02.10 defines, and the designer's
own validation panel, belong to those tasks.

`--workspace <file>` is where this milestone's sixth acceptance line lives: "same-named files are
told apart by root/package, removing a member deletes no directory and leaks no trust, no undo and no
target". A workspace is a directory with a `workspace.json` that names its members, each of which is a
root of its own with its own project document — deliberately **not** a second project document, because
pages, targets and services belong to a member and a copy of them here would be the second writable
model the project forbids. The identity rule is the whole point of the type: a file is
`member:path`, never a path, because two members that both name `pages/main.json` name two different
files; the file list is one row per (member, path) even though a page document is named by `pages`,
`startup` and a generated file's `from`. A path that leaves its member and stays in the workspace is
not refused but *qualified* — the member's own reader refuses it (it may not leave its root) and the
workspace answers the second question: this one lands in member `shared`, that one leaves the
workspace, and the third is inside the workspace with no member owning it, which is exactly what
adding a member answers. Members may not nest, because a path would then belong to two of them.
Adding and removing are edits to that one document: adding names a root that is *already there* (a
member that points at nothing has no answer to any question about it, and making directories belongs
to the creation path), removing never touches the directory — a test compares the directory's files
byte for byte afterwards — and removing is blocked while another member's document references a file
in it, with the references listed rather than silently fixed. The write is the only write: the
document on disk must still be the one the plan was made from, and then one atomic write goes in.
Which member is *active* is this machine's state, not the document's, for the same reason the active
target is: it is what somebody is working on now, so the shared document does not change when they
switch, and a recorded choice that names a member which is no longer there is reported rather than
moved to another member. Trust and targets are per member because they are keyed by the member's own
root and the member's own identity — there is no second notion of either here. A member whose
directory somebody moved is still the member that owns its paths, and it says so. The tree panel, the
member explorer and the drag-and-drop that would drive this belong to the shell (T04/T09); reading
`Cargo.toml` to discover members, features and the dependency tree belongs to T02.11.

Documentation checks
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

## Publishing

`rustage` sets `publish = false` in `Cargo.toml` on purpose, and `cargo publish` failing with
"`package.publish` must be set to `true`" is that setting working, not a defect. Two reasons are
recorded in the manifest itself: this crate is the application rather than a library anyone would
depend on, and the project distributes packaged builds (roadmap F25/T25.02). Making it publishable
would need more than flipping the flag — the pinned `rust_widgets` dependency is a git revision, and
a registry refuses git dependencies — so that path starts with publishing the widget library, which
is a decision for the project owner rather than a build fix.

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

## Requirements, release scope and gates

[`docs/plan/traceability.md`](docs/plan/traceability.md) connects each requirement from the
whitepaper's goals and the roadmap's architecture boundaries to the journeys, tasks, gates and
measurement scenarios behind it, with the data in
[`docs/plan/traceability.json`](docs/plan/traceability.json). The check refuses a requirement with no
owning task, no journey or no gate; refuses a gate declared as automatic that the workflow does not
actually run (or a manual gate with no reason and owner); and refuses a baseline task the roadmap
does not define. A gate that is declared but never run is the failure this exists for.

## Capability supplements

Anything the roadmap adds beyond its first delivery - multi-root workspaces, headless and offline
builds, staged hot update, the dual-AI workspace, and the extensions kept out of the release - is
recorded in [`docs/plan/capability-reference.md`](docs/plan/capability-reference.md) with the data
in [`docs/plan/capability-reference.json`](docs/plan/capability-reference.json). Every entry states
its source, its decision (adopt, defer or not-do), why, and the tasks, journeys and gates that carry
it, plus the milestone that signs it off. The check derives each task's milestone from the roadmap's
own milestone table, so a dependency that lands after the sign-off phase fails.

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
