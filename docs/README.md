# Documentation

## Suggested reading order

1. [Project overview](../README.md): current status and scope.
2. [Principles](plan/principle.md): engineering constraints.
3. [Whitepaper](plan/whitepaper.md): architecture, output modes, and hot updates.
4. [Full roadmap](plan/roadmap.md): features, menus, tasks, and milestone acceptance.
5. [Support matrix](plan/support-matrix.md): the three hosts, five profiles and per-cell evidence
   level, with the machine-checked data it is rendered from.
6. [Decision register](plan/decisions.md): the conclusions for Q1–Q6 and the project-format,
   generated-output, upgrade and component-expression policies, with alternatives, compatibility
   impact and the values that remain undecided and owned by the project owner.
7. [Shared contracts](plan/contracts.md): the document model, change sets, permissions, versions,
   diagnostics and source mapping, with the invariants that are actually enforced and the evidence
   behind each of them.
8. [Journey walkthrough](plan/journeys.md): the seven mode prototypes and every step of roadmap
   section 34 with its entry and owning task.
9. [Benchmark environment](plan/benchmark.md): the fixed data sets, the recorded hardware baseline,
   the measurement protocol, and the 18 performance scenarios with their roadmap thresholds.
10. [Traceability and gates](plan/traceability.md): each requirement with its journeys, owning tasks,
   gates and measurement scenarios, plus the release baseline.
11. [Contribution guide](../CONTRIBUTING.md): workflow and checks.
12. [GitHub administration](github.md): owner-managed repository settings.

Related design evidence: [Capability planning references](plan/capability-reference.md)
records the supplementary requirements (adopted, deferred and refused with reasons), the protocol
versions this build speaks, the milestone that signs each of them off, and the future-extension
boundaries. It informs roadmap scope, not claims of implemented capabilities.

[Icon and resource plan](plan/resources.md) defines host/project resource boundaries,
source placement, rust-widgets integration constraints, and packaging acceptance.
Its host source layer is implemented in `resources/` with checks in `tools/resources/`;
the icon seam and the control-level strip are `src/resources/` and `src/app/shell/toolbar.rs`,
with baselines under `tests/baselines/icons/`.

[Issue record](issues/issue-20261013.md) tracks the icon, resource and packaging gaps and the
order they are being closed in. It is a work list, not a statement of implemented capability.

## Records

- [MS03 continues: the command palette, the workbench, panels and batch operations](log/log-20261015-8.md):
  `app::palette` (fuzzy search over the registry, so a disabled hit carries the registry's own reason
  and recents nudge without hiding anything new), `app::workbench` (the title derived from the active
  document, project and target, so the three identities cannot disagree with the state, with the mode
  deciding the regions), `app::panels` (a registered panel whose service is missing is not shown at
  all, and hiding and showing restores its size and focus), and `app::operations` (a batch operation
  whose inapplicable items are refused rather than skipped, with the skip a recorded choice and the
  permission explained before it is checked), and `app::audit` (the registry, menu tree and keymap
  walked against each other, so nothing dangling is reachable and a new area re-verifies through the
  same walk), `app::tabs` (a tab is a view and a document is what it shows, so closing one and closing
  the other are different operations), and `app::focus` (one keypress produces one route that is claimed
  unless nothing was bound, and the menu, toolbar and context menu all read the registry so they cannot
  disagree) — 56 new tests (632 total).
- [MS03 begins: a typed command registry, the menu tree and shortcuts](log/log-20261015-7.md): `app::command`
  (every command described once, with an `Invocation` that has no public constructor, so an unauthorized
  or badly-parameterised command cannot bypass the menu and run), `app::menu` (the §3 tree bound to the
  registry, where an implemented leaf must name a registered command, no two leaves may share one id, and
  a leaf marked "planned" must not hide a command that works), and `app::shortcuts` (scopes, two-key
  chords, conflict detection that reports rather than orders, and a user keymap that round trips) — with
  the text-input guarantee that a typed character, or an input-method composition, never fires a global
  command, 28 new tests (576 total).
- [MS02's tail: a three-way conflict, recovery copies, the close decision and a retention
  policy](log/log-20261015-6.md): `project::conflict` (a line-level diff3 merge that never picks a
  side silently, read-only status, and a deleted file), `project::recovery` (copies kept outside the
  project, each carrying its own provenance, offered at startup and refusable), `project::closing` (a
  dirty-content decision whose cancel and failed write both give the state back unchanged), and
  `project::retention` (a prune that may only ever touch the store, and never a document's only
  version) — with the MS02 per-task review and sign-off, 26 new tests (548 total), and one real
  defect fixed (second-resolution timestamps could not order copies written in the same second).
- [The first macOS build, saving a document through a temporary, and three macOS
  findings](log/log-20261015-5.md): the dependency re-pinned from `58012c05` to `8178d048` because the
  pinned revision does not compile on macOS (an `i32`/`bool` cast in its file picker, already fixed
  upstream), `project::save` implemented (task T03.04: save/save-as/save-a-copy and a set committed
  whole, through a temporary and a rename, with the disk-full/permission/interruption paths pinned by
  tests), the external-modification guard T03.05 asks for added to the save path, and the three macOS
  findings — the cocoa event loop never ticks so a bounded `--launch` session never ends, FSEvents
  reports a deletion as `Modified`, and two concurrent runs of the host-readiness report deadlock (the
  test suite's own fix) — with the first two filed as upstream requests.
- [Two host-side workarounds migrated, `open_url` kept with a written reason, and one new upstream
  request](log/log-20261015-4.md): `host::watch` moved onto the library's `ChangeKind` so a deletion
  and a rename are observable (closing the input T03.05 needs, pinned by a test that could not have
  passed before the re-pin), `host::clipboard` moved onto `clipboard_text() -> Option<String>`, and
  `host::open_url` deliberately **kept** — the library's `open_external` is fire-and-forget and
  exposes no handler-availability query, so adopting it would trade two verified probe facts for
  nothing (principles #38/#45); the missing query is filed as an upstream request.
- [The dependency re-pin, a closed editor gap, and a bounded session that now stops
  itself](log/log-20261015-3.md): the dependency raised from `c957fdec` to `58012c05` — the revision
  that implements every finding in `rw-feedback/` — with the build and all 510 tests passing
  unchanged, the last `--probe-apis` gap closed by asserting the library's
  `line_state_is_known`/`resolve_line_state` pair, the host probe's file-dialog check moved onto the
  `capabilities().file_dialog` query, the bounded `--launch` session migrated from
  `std::process::exit` onto the library's own `quit()`, and the five host-side workarounds the new
  revision obsoletes registered as follow-up work rather than half-migrated.
- [MS00 and MS01 acceptance review, a real Linux desktop session and a probe that ended its own
  report](log/log-20261015-1.md): the per-task re-check of all twenty-nine MS00/MS01 tasks against
  their acceptance lines, the six defects it found and fixed (the stage's own lint debt, the
  toolchain report that merged "not installed" with "did not answer", the test directories the
  network tests left in `/tmp`, the flattened `--help` block, two exit codes that meant two things
  for one finding, and one evidence line whose geometry did not match the session it described), the
  first run of the host probe in **native** GTK session (and the host defect it exposed: the probe
  used to end its own report), and the two stage sign-offs with their boundaries.
- [Windows cross-build, a Wine run, and three cross-platform defects](log/log-20261015-2.md): what
  "can Wine replace a real Windows machine?" resolves to — a real cross-build for
  `x86_64-pc-windows-gnu` (the Windows host row is now `build-verified`), the same binary's
  `--probe-host` run under Wine 9.0 (`9 ready / 4 gap / 2 not-verified`, and why those numbers are
  about Wine rather than about Windows), the three defects the two runs exposed (`grace` unused off
  unix, three unix-only test imports, and a `PATH` lookup that could not see `cmd.exe`), and the four
  checks that still need a shell to drive their controlled process.
- [Resource structure and files](log/log-20261013-1.md): what was added, the commands that ran,
  and what remains unverified.
- [Glyph design, theme contract and HarmonyOS packaging](log/log-20261013-2.md): the minimal
  monoline glyph language, the verified HarmonyOS desktop manifest and resource layer, and the
  evidence each of them rests on.
- [Icons reaching real controls](log/log-20261013-3.md): the pinned dependency, the semantic icon
  seam, the eight roles mapped onto the dependency's own theme, the real-control baselines, and
  what is still not verified (a window, interaction states, RTL).
- [Host platform services, the painted chooser handoff and the host readiness report](log/log-20261014-1.md):
  the five services T01.02 asks for (file dialogs, the clipboard, system paths, URL opening and file
  events), the background executor T01.03 asks for, the owned subprocesses and directed tree cleanup
  T01.04 asks for, project trust, permissions and the authorization record T01.05 asks for, and the
  notification centre T01.06 asks for, the project registry with its shutdown order T01.07 asks for,
  and the credential references, TLS configuration and network permits T01.09 asks for (with the
  decision-table row that change required), the project format version 1 T02.01 asks for, whose
  specification is the module documentation of `src/project/record/format.rs` and whose example is the
  committed project under `examples/demo-project/`, and the page document format and the creation
  path T02.02 asks for, which write a project into a staging directory and put it in place with one
  rename, and opening a project T02.03 asks for, with the trust question, the machine-local recent
  list and the toolchain check that runs only under a permit, and the project format version 2
  T02.04 asks for, with its versioned migration step, the entry a target starts with, the classified
  project tree, the rule that removing an entry fixes the references to it or blocks, and the file
  operations T02.05 asks for, whose plan reads the project without writing and whose failure puts
  the moves it already made back, and the build configuration T02.06 asks for, whose every value
  carries the layer it came from and whose machine-local choices belong to one project, and the
  import T02.07 asks for, whose candidate is a diff before anything is written and whose failure
  keeps the file that was read, and the validation and re-scan T02.08 asks for, which answers every
  need with present/missing/not-checked/refused and prints the project's own bytes unchanged as its
  last line, and the workspace T02.09 asks for, whose files are identified by the member that names
  them and whose member removal touches no directory, and the versioned kit T02.10 asks for, whose
  members declare the triples they can serve and which is asked about a triple rather than a
  profile, and the dependency resolution T02.11 asks for, whose command and effects are one value
  authorized class by class and whose default resolution cannot write a file or reach the network, and
  the buffer T03.01 asks for, whose invalid text is kept and whose last valid model can still be
  previewed, and the batch of edits T03.02 asks for, which lands whole or not at all, is prechecked
  before it runs, and reports what would undo it; the
  environment this round's evidence came from — a Linux preview/stub backend, not the macOS session
  the previous record measured — the pre-existing problems found before starting and how each was
  repaired, the defects the round's own tests turned up, the upstream gaps the round found and
  recorded, and every check `--probe-host` performs together with what it still cannot verify.
- [The three-desktop probe record](plan/host-probe.md): which of the host's minimum capabilities —
  startup, input, process and file, plus the platform services and the project scope it also has —
  were measured on which desktop, with the Linux column's real results and the macOS and Windows
  columns marked blocked for want of a test machine.
- [Dependency contract probes, a pinned toolchain, the editor surface, the support matrix, the decision register, the shared contracts, the journey walkthrough and the benchmark environment](log/log-20261013-4.md):
  the fixed Rust toolchain, the re-checked dependency policy, runnable probes over the layout,
  generation, event, unbind and text-editor APIs, the revision the report is pinned to, the one
  library-layer gap the editor probes found, the three-host by five-profile support matrix, and the
  Q1–Q6 plus format/generated-output/upgrade/component-expression decisions with their rejected
  alternatives and the values still owned by the project owner, and the eight document contracts
  that make "no second writable model" a checked property rather than a promise, and the seven mode
  prototypes with every roadmap-journey step mapped to an entry and an owning task, plus the fixed
  fixture sizes, the recorded hardware baseline and the measurement protocol behind the performance
  scenarios.

Upstream findings raised against [rust-widgets](https://github.com/mikewolfli/rust-widgets) are kept
in [`rw-feedback/`](rw-feedback/feedback-20261013.md): the first file records what was verified and
what, if anything, was requested upstream against the pinned revision, and now carries a per-section
status after the re-pin; the second compares every one of those findings with the library's local
`HEAD` (all of them are implemented there, and the one new request — the GTK backend's `quit()`
calling `gtk::main_quit()` with no thread check — is implemented too) and records which of them the
2026-10-15 re-pin closed.

## Platform verification

[Everything that needs a real machine or device](platform/verification.md) gathers the items that can
only be settled on the platform itself — the three desktop hosts, the fifteen host-by-profile cells,
the HarmonyOS 2in1 track, the performance baseline, and the upstream items that needed re-checking
after the 2026-10-15 re-pin (which has since been done, so what remains is the five host-side
workarounds the new revision obsoletes) — each with what is missing, the command to run, where the
result goes, and a mark for the ones that already have real-machine evidence (the macOS baseline
session, the Linux probes in both build shapes, both host builds, and the Windows cross-build).

## Boundaries

Community documents and GitHub templates primarily use English.
The README is bilingual; the design documents in `plan/` use Chinese.

Design documents are not claims of implemented features.
Historical upstream evidence in the whitepaper must be checked against the
actual revision before implementation.
Completing a roadmap task requires implementation and acceptance evidence,
not merely checking off its description.

Upstream material belongs to
[rust-widgets](https://github.com/mikewolfli/rust-widgets).
This repository does not copy upstream code or count library capabilities as
completed designer features.
Upstream default-branch documents may change; record commit SHAs for implementation baselines.

Add new documents to this index and applicable README sections.
Keep both README versions consistent when project status or scope changes.
