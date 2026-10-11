# GitHub Repository Administration

## Files provided

| File or directory | Purpose |
|---|---|
| [LICENSE](../LICENSE) | Standard MIT license |
| [README.md](../README.md) / [README.zh-CN.md](../README.zh-CN.md) | Bilingual entry points |
| [CONTRIBUTING.md](../CONTRIBUTING.md) | Contribution and validation workflow |
| [CODE_OF_CONDUCT.md](../CODE_OF_CONDUCT.md) | Community policy and reporting channels |
| [SECURITY.md](../SECURITY.md) / [SUPPORT.md](../SUPPORT.md) | Private reporting and public support boundaries |
| [CHANGELOG.md](../CHANGELOG.md) | Actual changes, not planned release features |
| [Issue forms](../.github/ISSUE_TEMPLATE/) | Bug, feature, design/governance, and compatibility reports |
| [PR template](../.github/PULL_REQUEST_TEMPLATE.md) | Scope, evidence, permissions, and documentation |
| [CODEOWNERS](../.github/CODEOWNERS) | Current ownership; update as maintainers join |
| [Documentation CI](../.github/workflows/docs.yml) | Checker tests and repository documentation checks |
| [Checker](../tools/check-docs.mjs) | Local and CI entry point without npm dependencies |

Community policies and GitHub templates primarily use English.
The README is bilingual; project design documents remain in Chinese.

## What CI actually checks

Use Git and Node.js 22 or newer. The workflow runs on push, pull request,
and manual dispatch. It grants only `contents: read`, requires no repository
secrets, and has no publishing or deployment steps.
Official Actions are pinned to commit SHAs; review updates before changing them.

```sh
node --test tools/check-docs.test.mjs
node tools/check-docs.mjs
```

Checks cover tracked and non-ignored new documentation, configuration, and tool
files for whitespace and conflict markers, relative Markdown file/directory
links, roadmap task IDs and dependencies, and required community files.

The checker does not fetch external URLs, validate heading anchors, parse full
Markdown/YAML syntax, or replace design review or target builds.
Confirm issue forms render correctly on GitHub after merging them.
Local uncommitted files participate; ignored files do not.

There is no Cargo project yet, so CI does not pretend to compile Rust, collect
application coverage, publish packages, or deploy an application.
Add those workflows after the actual baseline decisions and implementation.

## Owner-managed GitHub settings

Local files cannot enable the settings below. No remote settings were changed:

1. **Description and topics:** describe a Rust desktop visual UI designer.
   Suitable topics include `rust`, `gui`, `ui-designer`, and `desktop`;
   do not imply a released application exists.
2. **Issues:** enable them; templates become available after they reach the default branch.
3. **Actions:** allow official Actions and keep default token permissions read-only.
4. **Private vulnerability reporting:** enable it in security settings; otherwise
   the private reporting URL may be unavailable.
5. **main ruleset:** require pull requests, the `Documentation checks` status,
   and resolved review discussions. Require code-owner approval only when the
   team can actually provide it; do not force an impossible sole-maintainer self-review.
6. **CODEOWNERS:** verify `@mikewolfli` has the required permissions.
   This file does not grant access.
7. **Merging:** squash merging and automatic topic-branch deletion are recommended.
8. **Labels and milestones:** create them manually or through a separately
   reviewed operation. This scaffold does not create 224 issues or invent due dates.
9. **Discussions and project boards:** enable separately if useful; their existence
   is not assumed.
10. **Contact and release policies:** publish an appropriate private conduct
    contact and document supported versions before the first application release.

Suggested labels: `bug`, `enhancement`, `documentation`, `design`,
`compatibility`, `blocked`, and `needs-evidence`.
Forms do not automatically apply labels that might not exist.

## Future evolution

Introduce real `Cargo.toml`, source, tests, and toolchain settings only when the
Rust host implementation starts.
Then add caching, three-desktop build matrices, output-target verification,
and dependency updates for the established project.
Keep signing and device credentials in authorized secret references, not Git files.
