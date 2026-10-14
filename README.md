# rustage

**A desktop visual UI designer and development workbench for Rust, powered by
[rust-widgets](https://github.com/mikewolfli/rust-widgets).**

[English](README.md) | [简体中文](README.zh-CN.md)

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

## Project status

rustage is currently in the **design and planning stage**. The repository now
includes a minimal Rust executable scaffold, but it does **not** yet contain a
runnable visual designer or released installation packages. The scaffold pins a
single third-party dependency — [rust-widgets](https://github.com/mikewolfli/rust-widgets),
at a fixed revision and the `desktop` profile — and a fixed Rust toolchain, and it
carries runnable probes that exercise the dependency's layout, generation, event
and unbind contracts. It has no designer window.

Capabilities below are planned, not claims of implemented or verified support.
The roadmap defines 29 functional domains, 262 implementation tasks,
20 milestones, 26 user journeys, and their acceptance criteria.
The formal release scope covers 28 domains, 251 tasks, and 23 journeys.
Advanced native/composite debugging, integrated profiling, and remote/container
capabilities are future extensions, not release dependencies.

## What rustage is designed to do

- Design pages, components, layouts, properties, CSS styles, and resources.
- Edit application menus, shared actions, toolbars, dialogs, and collection content.
- Connect typed data, events, state, navigation, and Rust business logic.
- Keep the canvas, object tree, source views, and diagnostics in one project context.
- Preview valid UI changes without recompiling Rust.
- Generate deterministic Rust source while preserving declared handwritten regions.
- Integrate project editing, language services, build/run, debugging, tests,
  version control, and deployment through host services.
- Offer optional, explicitly authorized AI change proposals without making AI
  necessary for normal development.

## Desktop host, multiple output targets

The designer itself runs only on desktop. macOS, Windows, and Linux host
support will be validated separately.

Output profiles are distinct from operating systems and Rust target triples:

| Output profile | JSON/CSS runtime output (mode 1) | Static Rust output (mode 2) |
|---|---|---|
| `desktop` | Planned | Planned |
| `tablet` | Planned | Planned |
| `mobile` | Planned | Planned |
| `mini` | Not applicable | Planned |
| `embedded` | Not applicable | Planned |

Actual support depends on library capabilities, toolchains, SDKs, and target
tests. Desktop simulation is not device certification.

### Planned hot-update model

1. **UI and existing bindings:** validate and apply JSON/CSS/model changes.
2. **Managed Rust business logic:** incrementally build a candidate and replace
   a separate logic process through a versioned protocol and explicit state migration.
3. **Incompatible changes:** request a restart or a confirmed state reset.

This is not arbitrary in-process Rust code replacement. Development IPC and
file-watching services must not leak into restricted release artifacts.

## Documentation

| Document | Purpose |
|---|---|
| [Documentation index](docs/README.md) | Reading order and documentation boundaries |
| [Whitepaper](docs/plan/whitepaper.md) | Architecture, output modes, and hot-update design |
| [Project principles](docs/plan/principle.md) | Engineering constraints and review criteria |
| [Complete roadmap](docs/plan/roadmap.md) | Product modules, menus, tasks, milestones, and acceptance |
| [Support matrix](docs/plan/support-matrix.md) | Three hosts, five profiles and per-cell evidence level, with the data file behind it |
| [Decision register](docs/plan/decisions.md) | Q1–Q6 and the format, generated-output, upgrade and component-expression policies, including what is still undecided |
| [Shared contracts](docs/plan/contracts.md) | The one writable document model, change sets, permissions, versions, diagnostics and source mapping |
| [Journey walkthrough](docs/plan/journeys.md) | The seven mode prototypes, and every roadmap journey step with its entry and owning task |
| [Benchmark environment](docs/plan/benchmark.md) | Fixed data sets, hardware baseline, measurement protocol and the performance scenarios |
| [Traceability and gates](docs/plan/traceability.md) | Requirements mapped to journeys, owning tasks, gates and measurement scenarios |
| [Icon and resource plan](docs/plan/resources.md) | Resource placement, icon integration, themes, and platform packaging |
| [Capability planning references](docs/plan/capability-reference.md) | Workflow requirements, protocol evidence, and future-extension boundaries |
| [GitHub administration](docs/github.md) | Repository settings and CI setup still requiring owner action |

Community documents and GitHub templates primarily use English; design documents
are currently written in Chinese. Both README versions should remain consistent
when project status or scope changes.

## Repository layout

```text
rustage/
├── .github/              # Issue forms, pull request template, ownership, CI
├── docs/
│   ├── plan/             # Principles, whitepaper, and roadmap
│   ├── README.md         # Documentation index
│   └── github.md         # Repository administration guide
├── src/                  # Executable entry point, CLI boundary, and dependency contract probes
├── Cargo.toml            # Rust package manifest
├── Cargo.lock            # Locked Rust dependencies
├── rust-toolchain.toml   # Pinned Rust toolchain
├── tools/                # Dependency-free documentation checks and tests
├── README.md             # English overview
├── README.zh-CN.md       # Chinese overview
├── CONTRIBUTING.md       # Contribution workflow
├── CODE_OF_CONDUCT.md    # Community expectations
├── SECURITY.md           # Vulnerability reporting policy
├── SUPPORT.md            # Questions, bugs, and support boundaries
├── CHANGELOG.md          # Verified repository changes
└── LICENSE               # MIT license
```

## Getting started

```sh
git clone https://github.com/mikewolfli/rustage.git
cd rustage
cargo run
cargo run -- --help
cargo run -- --probe-apis target/probe   # writes target/probe/api-probes.json; exits non-zero while a probe reports a gap
cargo run -- --probe-host target/host    # writes target/host/host-readiness.json; exits 4 while a host check is not ready
cargo run -- --project-check examples/demo-project/project.json   # reads a project document, writes its round trip next to it
cargo run -- --new-project target/new-project       # creates a project: plan, stage, one rename
cargo test
node --test tools/check-docs.test.mjs
node tools/check-docs.mjs
```

The Rust scaffold uses the 2024 edition. Its third-party dependencies are the
pinned `rust-widgets` revision, which Cargo fetches over the network on a cold
build, and `libc` on Unix only, and its Rust toolchain is pinned by
`rust-toolchain.toml`. Documentation
checks require **Node.js 22 or newer** and Git, with no npm dependencies. The
executable currently reports its status, supports help and version options, renders
the icon strip, runs the dependency contract probes, checks the host's own platform
services, and opens the bounded baseline desktop session. It does not implement the
designer: no document model, canvas, property panel, preview or generator exists yet.

## Contributing and support

Read [CONTRIBUTING.md](CONTRIBUTING.md) before submitting changes.
Use the issue forms for bugs, feature proposals, design decisions, and target
compatibility evidence. See [SUPPORT.md](SUPPORT.md) for routing questions.

All community participation follows [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).
Do not disclose vulnerabilities or secrets in public issues; follow
[SECURITY.md](SECURITY.md).

## License

rustage is licensed under the [MIT License](LICENSE).
Third-party dependencies, assets, fonts, and imported templates retain their
respective licenses and must be reviewed before distribution.
