# Contributing to rustage

[English overview](README.md) | [中文概览](README.zh-CN.md)

## Current stage

This repository contains design documents and contribution infrastructure,
not a runnable application. Contributions may include design reviews,
documentation fixes, reproducible upstream API evidence, and roadmap refinement.
Confirm the relevant baseline decisions before introducing a Cargo application,
dependencies, a toolchain, or new public architecture.

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

Use Git and Node.js 22 or newer. No npm packages are required:

```sh
node --test tools/check-docs.test.mjs
node tools/check-docs.mjs
git diff --check
```

Checks cover local Markdown file links, whitespace, conflict markers, required
community files, and roadmap task dependencies. They do not fetch external URLs,
validate heading anchors, or replace design review.

There is no Cargo application yet. Do not claim that `cargo check` or an
application run passed when no such project exists.
Add Rust formatting, compilation, tests, and target artifact checks when the
actual implementation and toolchain are established.

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
