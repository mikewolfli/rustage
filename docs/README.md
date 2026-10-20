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
order they are being closed in. [MS12 GUI entry work](issues/issue-20261020.md) tracks the
designer's canvas, property editor, command palette and debug panel as existing roadmap tasks that
still need their window entry; both are work lists, not statements of implemented capability.

## Records

- [2.8.11 re-pin, the MS12 journey walk and a page-to-generator seam](log/log-20261020-3.md): the
  dependency moved to `cee6b22367244527aa407bb9ae67305751f61c26` (2.8.11), which adds
  `BorderLayout`, panel child factories and the GTK resize repaint fix. The workbench now creates
  panel children through the panel (no implicit parent contract) and allocates its regions with
  `BorderLayout`; a `wmctrl` resize repaints without any input event. The MS12 harness
  (`tools/ms12-journeys/run.py`) drives 74 steps across J01–J09 and J17–J19 and separates real,
  model and blocked evidence. Two integration defects were found and fixed: page documents could
  not reach `--generate`, and a new project's root kind was not the generator's `window`. 2.8.11
  regresses the `embedded` profile (`E0599` on four gated `WidgetKind` variants), so the support
  matrix backs four Linux cells with 2.8.11 builds, marks the other cells `unverified` (not
  re-run on this host) and marks `embedded` `unsupported` until upstream fixes it. The same
  round started the MS12 GUI work: the workbench command palette is now a real panel with a
  line edit, fuzzy registry search, disabled reasons, row selection and Run/Close, with focus
  restored on close. The Inspector now edits a schema-checked title through a real line edit and
  Apply/Clear: a refused value keeps its text and leaves the revision unchanged, while a valid
  value advances it once. The workbench also has an object tree and a real canvas: one
  `CanvasState` supplies the Outline rows, hit testing and the selection, so a tree row and a
  canvas click select the same node; hidden and locked nodes are refused with the reason that
  applies, and a drag marquee selects every selectable intersection. The selection drives the
  Inspector, and an accepted title renames the tree node. Enter/Escape and double-click
  activation are blocked by the pinned GTK key/pointer path and are recorded in
  `feedback-20261020.md` §0.2/§0.3 rather than faked; the general widget-creation entry returning
  0 on the native desktop backend is recorded in §0.4, and the host uses the control backend's
  own Canvas route. The debug panel drives a real gdb DAP session on a worker thread: the panel
  reports the adapter-verified breakpoint, thread, stack frame and `n: i32 = 41`, and Step/Run/Stop
  are real commands. A list-box rebuilt from a main-loop timer did not repaint; the panel uses
  one label per row instead and the behaviour is recorded in §0.5.

- [MS10's remaining seams and the MS11 debug client](log/log-20261020-1.md): local device discovery
  now starts the platform's own tools (`xcrun simctl`, `adb`), SDK detection reads the real
  installations (iOS 26.2, Android android-36), signing runs `codesign --force --sign` and verifies
  it, and the generated source for the restricted profiles builds and runs in independent consumer
  crates (`generated-mini-ok`/`generated-embedded-ok`). The generated mobile source also cross-builds
  for `aarch64-linux-android` with NDK 29. MS11's F19 is implemented as a real DAP client
  (`--debug`) with breakpoints, paged inspection, source mapping, hot-update exclusion and recovery;
  its transport is exercised against a separate DAP process, and a real `lldb-dap` session on macOS
  stops on a breakpoint, reports `n = 41`, steps out, continues to `value 42` and exits 0. A real
  attach to an external `sleep` process disconnects without terminating it.

- [Upstream re-pin and the MS09/MS10 remaining closures](log/log-20261019-2.md): the dependency
  moved to `504698fec4028b2ddb96609e517d145ea30a25e2` (2.8.9), and the host workarounds the old
  revision needed were removed — the library now moves router focus on a pointer press, grabs GTK
  focus on `map`, injects `ValueChanged`, and owns the toolkit input-method context. MS09's
  pause/drain swap is now driven against two real processes (`pause`/`drain`/`resume` on the worker,
  every `Swap` phase observed, and a zero-limit pause leaves the old generation serving). MS10's
  Windows desktop combination was built for real with `x86_64-pc-windows-msvc` on the macOS host
  using `rust-lld` and the xwin-extracted MSVC SDK/CRT (PE32+ `rustage.exe`); the matrix now records
  11 build-verified cells and 4 unverified. The generation pipeline also got its one explicit write
  point (`--generate write:<path>`), `tools/generated-project/run.py` builds and runs an
  independent consumer crate from that source (`generated-ui-ok`), and the terminal's byte pump is a
  real POSIX pseudo-terminal (`host::pty`, `--terminal exec/…/input/read/wait`), with a Windows
  ConPTY path compiled via `portable-pty` (not run on Windows here). The macOS host also
  ran a real cocoa `NativeInteractive` 1280x800 bounded session. Local delivery now detects iOS 26.2
  and Android android-36 SDKs, discovers 11 iOS simulators through `xcrun simctl` (Android `adb`
  reported 0 running devices), and signs a real Mach-O with `codesign` (`codesign -v` exit 0);
  installation itself is still a plan. The language service also has a real stdio LSP transport: it
  starts `rust-analyzer`, completes `initialize`, reads the server's own capability set, turns a real
  `textDocument/completion` result into typed items, and shuts it down bounded. The design
  workbench also has a real window now: `--workbench-gui` opens docked panels, a toolbar, two
  editor groups with tabs, a status bar, and saves/loads the layout through the shell models;
  drag-docking and floating panels remain. Recorded in `docs/rw-feedback/feedback-20261019.md`
  that the §17 requests are closed upstream.

- [Review of blue1–blue10 and the real two-process hot-update walk](log/log-20261019-1.md): the
  review found the milestones signed and no unimplemented task in range, fixed a support-matrix
  document that still claimed every cell was `unverified` while its own table listed build evidence,
  and closed MS09's two declared gaps — `host::subprocess::DuplexProcess` (a permit-gated, bounded
  request/response child), `app::logic::worker` (the replaceable side of the typed channel, which
  checks its own generation), `--logic-worker`/`--logic-process` (two real generations over pipes,
  with the retired generation refused and a `preserve` snapshot/restore that carries the old state
  and its fields into the new generation), and `--logic-candidate build:<manifest>` (a real `cargo
  build` into a separate target directory; `fixtures/logic-candidate` includes the designer's own
  `core`/`ipc`/`worker` sources rather than copying the protocol). Linux host build/run evidence
  moved `linux × desktop` to `build-verified` and the Linux designer host to `certified` — 22 new
  unit tests and 9 real-process integration tests. Schema-level state migration now runs over the
  process boundary (`first-schema`/`second-schema`): a lossy migration blocks the switch and leaves
  the old generation serving, and a generation whose own schema refuses the import is not adopted
  either. A bounded GTK session was checked with `xwininfo`: a real 1280x800 window is mapped and
  its paint callback fills the client area with the clear colour (the first attempt exposed a host
  defect where only the widget model, not the OS window, was created). `--smoke` draws a real
  button and field; `--smoke-worker` forwards each real XTest click through a channel to a real
  `--logic-worker` process and logs the toggling `running`/`idle` state. `tools/gui-smoke/run.py`
  checks click, drag-cancel and the worker toggles. The keyboard/IME workarounds described here
  were removed after the upstream re-pin: the library now owns pointer focus, GTK focus and the
  toolkit IM context, so `run.py --keys`/`--ime` exercise the library path directly (see
  [log-20261019-2.md](log/log-20261019-2.md)). Old feedback §17 was closed upstream; design-time
  event-binding panels and device targets remain unverified.
- [MS10: multi-target projects — target configuration, the versioned capability snapshot, the local
  desktop matrix, the restricted profiles and the certification report](log/log-20261016-4.md):
  `app::targets` (`configuration` — profile, triple, toolchain, SDK, mode and runner, with a profile
  that is not an OS and no mode 1 on mini/embedded; `snapshot` — a versioned four-level capability
  answer, a switch that marks instead of deleting; `matrix` — four committed desktop combinations
  with independent artifacts and a success that needs a run; `devices` — tablet/mobile sizes, touch
  targets, resource capacity, and simulator/device evidence that is never averaged; `restricted` —
  static/`no_std` applicability, the three capacity boundaries and a link audit over the real
  `cargo tree` closure plus a gate only a real build passes; `local` — local tools with a distinct
  "not applicable" state, package export that reads itself back, install plans and recovery;
  `queue` — per-job failure that keeps the matrix from looking green, measured artifacts and
  credential references that cannot hold a secret; `certification` — five stages per profile with
  evidence-required cells and the F17 IPC/static consistency measurement), the global
  `--target-*` commands, and the support matrix updated from real evidence — 8 tasks, 92 new tests,
  with Linux/Windows runs, mobile SDKs/devices and linked generated artifacts explicitly
  unverified. Recorded in the same file as MS07–MS09 and the host-UI i18n work.
- [Host interface i18n begins — the identified string vocabulary, the catalog and the generated English
  language pack](log/log-20261016-4.md): `host::locale` (the known languages, the system's language read
  and the `bundled` fact), `app::locale` (`catalog` — a `Pack` that round-trips through its file form, a
  `Catalog` whose missing key answers with the key rather than nothing, and a `select` that refuses an
  explicit unbundled request while falling back from the system; `vocabulary` — the interface strings
  identified from the real command registry, menu tree and keymap, plus the diagnostic templates and the
  shell lines, with the English pack *derived* from it so the two cannot drift), the global
  `--ui-language <tag>` and the `--locale-report <directory>` producer, and the diagnostics
  (`Refusal::message`, `why_not_exists`) plus the status/version lines now read through the catalog —
  73 identified strings, 26 new tests. Done ahead of its milestone at the user's request; only English is
  bundled, and what T24.03 still owes is listed in the record. Recorded in the same file as MS07–MS09.
- [MS09: Rust event-logic hot update — a shared business core, typed local IPC, the candidate build,
  state migration, the controlled replacement and the vector regression](log/log-20261016-4.md):
  `app::logic` (`core` — a pure event/state → state/action reduction whose seam is a value, not a live
  channel; `ipc` — a versioned, bounded, text-only channel with a handshake, a manifest and a
  generation guard; `actions` — synchronous and high-frequency events kept in the UI and a bounded
  queue whose overflow names the item it dropped; `candidate` — a debounced incremental build whose
  `check` is not a candidate and which refuses to overwrite the running binary; `migration` — a
  per-field migration preview that blocks the automatic switch on a real loss; `swap` — the ordered
  pause/drain/snapshot/import/confirm/resume/retire walk with a bounded pause; `combination` — the
  style-changes-need-no-build rule and late-result isolation; `regression` — the event vectors driven
  over both call seams and the fault decisions that do not replay) — 8 tasks, 67 new tests. Recorded
  in the same file as MS07 and MS08, as requested.
- [MS08: full application-design productivity — components, form validation, the target app shell
  and collection controls](log/log-20261016-4.md): `app::component` (definitions, instances and cycles,
  authoring, variants with their inverses, custom-widget publishability, resource import/replacement,
  per-target variants and capacity, package import/export, definition upgrade and the asset
  manifest), `app::data::validation` (typed rules with no script, cross-field rules, submit/reset,
  async cancellation and staleness), `app::application` (the *target* app's shell, shared actions,
  menus, tool bars, context menus, dialogs, dock and the end-to-end walk) and `app::collection`
  (content model, table columns, tree items, cell delegates, navigation containers, rich content,
  specialised views and the large-collection walk) — 26 tasks, 238 new tests. Recorded in the same
  file as MS07, as requested.
- [MS07: the first delivered source — deterministic generation and a real build/run](log/log-20261016-4.md):
  `app::generate` (the plan, the pipeline over the library's own generator, the node-to-source map,
  the merge that keeps hand-written regions, the reverse reader, the position recalculator, the
  staged export and the golden comparisons) and `app::build` (the toolchain probe, check/build/
  rebuild, the problem parser that links a compiler message to a node, the standalone run, tasks,
  terminals, external tools, the end-to-end walk, the typed task graph with a real problem matcher,
  reproducibility and the headless CI walk) — with the exact cargo argv printed, a real
  `cargo check` run through `--compile execute`, and the same input generating byte-identical source.
- [MS06 begins: data, events and preview — bindings, routes, state, adapters, the preview service
  and inspection](log/log-20261016-3.md): `app::data` (typed bindings, event declarations apart from
  their wiring, dispatch, routes, the state machine, adapters and the session log) and `app::preview`
  (the service, live update, state keeping, the viewport matrix, inspection, baselines, recovery and
  load), plus the `run.rs` split into `app::run/` — 16 tasks, 247 new tests.
- [MS05 begins: basic visual design — canvas, layout, properties and CSS, and the whole of
  blue5/MS04](log/log-20261016-2.md): `app::canvas`, `app::layout` and `app::style` (24 tasks) over the
  object tree, the layout contract, the property schema, the CSS view, tokens, states and motion.
- [MS03 ends and MS04 begins: the workbench, commands, layout, windows, chrome and in-place text
  editing](log/log-20261016-1.md): `app::groups`, `app::clipboard`, `app::layout`, `app::windows`,
  `app::chrome` and the editor, language-service and search families of MS04 (blue5).
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
