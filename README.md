# rustage

**A desktop visual UI designer and development workbench for Rust, powered by
[rust-widgets](https://github.com/mikewolfli/rust-widgets).**

[English](README.md) | [简体中文](README.zh-CN.md)

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

## Project status

rustage is currently in the **design and planning stage**. This repository
contains the architecture, engineering principles, implementation roadmap,
and contribution infrastructure. It does **not** yet contain a runnable
designer, a Cargo application, or released installation packages.

Capabilities below are planned, not claims of implemented or verified support.
The roadmap defines 28 functional domains, 224 implementation tasks,
20 milestones, and their acceptance criteria.

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
node --test tools/check-docs.test.mjs
node tools/check-docs.mjs
```

Documentation checks require **Node.js 22 or newer** and Git, with no npm
dependencies. There is no `cargo run` command yet. Rust build/test workflows
will be added when a real application and confirmed toolchain exist.

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
