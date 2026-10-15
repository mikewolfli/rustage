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
the icon seam and the control-level strip are `src/resources/` and `src/app/toolbar.rs`,
with baselines under `tests/baselines/icons/`.

[Issue record](issues/issue-20261013.md) tracks the icon, resource and packaging gaps and the
order they are being closed in. It is a work list, not a statement of implemented capability.

## Records

- [MS00 and MS01 acceptance review, a real Linux desktop session and a probe that ended its own
  report](log/log-20261015-1.md): the per-task re-check of all twenty-nine MS00/MS01 tasks against
  their acceptance lines, the six defects it found and fixed (the stage's own lint debt, the
  toolchain report that merged "not installed" with "did not answer", the test directories the
  network tests left in `/tmp`, the flattened `--help` block, two exit codes that meant two things
  for one finding, and one evidence line whose geometry did not match the session it described), the
  first run of the host probe in **native** GTK session (and the host defect it exposed: the probe
  used to end its own report), and the two stage sign-offs with their boundaries.
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
  specification is the module documentation of `src/project/format.rs` and whose example is the
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
what, if anything, was requested upstream against the pinned revision; the second compares every one
of those findings with the library's local `HEAD` (nine of them are already implemented there) and
records the one new request — the GTK backend's `quit()` calls `gtk::main_quit()` with no
thread check.

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
