# Security Policy

## Supported scope

rustage is in the design stage, with no released application or maintained
binary version. Reports may concern repository tooling, GitHub workflows,
documentation, and implementation added later.
Supported release versions and maintenance policies will be documented before
the first application release.

## Private reporting

**Do not publish exploit details, credentials, or private data in issues,
pull requests, or logs.**

If private vulnerability reporting is enabled, use:

<https://github.com/mikewolfli/rustage/security/advisories/new>

Availability depends on owner-managed GitHub settings; local repository files
cannot enable this service.
If unavailable, check for a private contact explicitly published by
[@mikewolfli](https://github.com/mikewolfli).
If no private channel exists, request that one be enabled in a public issue
**without including vulnerability details**.

Include the affected revision, scope, environment, minimal reproduction,
expected and actual behavior, potential impact, and suggested remediation.
Share only necessary, sanitized data. Do not test systems you do not own or
upload projects without authorization.

## Response

Maintainers will assess impact, coordinate remediation and regression checks,
and determine appropriate disclosure.
The project does not currently promise a response deadline or a bounty.

If credentials are exposed, their owner must revoke or rotate them first.
Deleting a Git file does not revoke a credential.

## Planned trust boundaries

- Project Rust code, build scripts, terminals, external tools, and deployment
  have executable behavior or real side effects.
- Logic and extension processes are not security sandboxes.
- Sending AI context requires explicit authorization; secrets and ignored
  files must not be sent by default.
- Pull request workflows do not automatically receive publishing, deployment,
  or repository write permissions.
