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
  rename; the
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
in [`rw-feedback/`](rw-feedback/feedback-20261013.md); it records what was verified and what, if
anything, was requested upstream.

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
