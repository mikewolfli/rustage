# Release guide

This guide documents the release operations the build actually has, and the steps it does **not**
implement. It is the deliverable for T25.06 (release tutorial and known limitations); it does not
claim signing, notarisation or an installer that this repository does not contain.

## 1. Freeze the inputs

```sh
rustage --release freeze:<commit>:<generator-version> \
                   format:desktop \
                   package:0.1.0 \
                   widget-revision:<pinned rev> \
                   document-format:1 \
                   protocol:1 \
                   support:<matrix revision> \
                   support-digest:<sha-256 of the matrix> \
                   freeze-check
```

`freeze-check` compares the frozen facts with the repository. A fact the repository contradicts is a
failure; a matrix cell the repository itself marks unsupported is reported as blocked rather than
pretended to pass.

## 2. Build and stage

```sh
cargo build --release
rustage --release freeze:<commit>:<generator> \
                   stage:target/release/rustage:dist/rustage-<version>
```

A stage is the directory an installer packages:

```text
dist/rustage-<version>/
├── rustage                 # the binary
├── language/
│   ├── en.json
│   └── zh-Hans.json
└── stage.json              # every file's bytes and SHA-256, plus the freeze facts
```

- `language/` is runtime data. A stage without a pack is refused: a release that offers a language
  and ships no catalogue would paint keys.
- `*.pseudo.json` files are development artifacts and are not staged.
- `stage.json` is deterministic for the same inputs; an installer can verify it before and after
  signing.

## 3. Artifacts and verification

```sh
rustage --release artifact:rustage-<version>-macos.dmg:<sha256>:<bytes>:unsigned \
                   verify-file:dist/rustage-<version>.dmg
```

`verify-file` recomputes the digest from the bytes on disk. An artifact whose digest does not match
is refused; a signature is either `signed`/`verified` or `unsigned`, and the release report never
turns one into the other.

## 4. SBOM and licences

```sh
rustage --release package-license:serde@1.0.0:MIT \
                   sbom-lock:Cargo.lock:dist/sbom.json \
                   sbom:dist/release-sbom.json
```

A lockfile package whose licence is unknown is reported; the SBOM writer refuses rather than
omitting it silently.

## 4a. Provenance (T25.07)

```sh
rustage --release freeze:<rev>:<gen> package:<ver> widget-revision:<sha> \
                   artifact:rustage:<sha256>:<bytes>:unsigned \
                   provenance:Cargo.lock:dist/provenance.json
# On another machine, with the downloaded artifact:
rustage --release verify-provenance:dist/provenance.json:./rustage
```

The report binds the artifact's name, digest, size and signature state to the frozen revision,
package/generator/widget versions and support matrix, and to the lockfile's own SHA-256 and package
count. `verify-provenance` re-reads both files and recomputes both digests; it does not trust the
report's arithmetic.

## 4b. Signing and notarisation (T25.02)

```sh
# macOS; `-` is an ad-hoc signature, a real release names a Developer ID
rustage --release sign:dist/rustage:-
# Windows
rustage --release sign:dist\rustage.exe:<certificate-subject>
# Linux: writes dist/rustage.asc
rustage --release sign:dist/rustage:<gpg-key-id>
# macOS notarisation (needs a notarytool keychain profile with Apple credentials)
rustage --release notarize:dist/rustage:<profile>
```

`sign` runs the platform tool through the command-permission gate with an argument array (never a
shell string), then runs the platform verifier; a signature that does not verify is an error. macOS
and Windows embed the signature, so the command prints the artifact's new size and SHA-256 and asks
the caller to declare `artifact:` again with those values. Linux writes a detached `.asc` and leaves
the artifact bytes unchanged. `notarize` submits with `notarytool --wait` and staples the ticket; the
host neither stores nor asks for the Apple credentials.

## 5. Updates and rollback

```sh
rustage --release update:<from>:<to> \
                   channel:candidate \
                   verify-update \
                   confirm \
                   begin-install \
                   complete-install \
                   journal:dist/update-journal.json
```

Every transition is recorded in the journal. An interrupted install is recovered with
`journal-recover:<file>`; `rollback` restores the previous version. A project-format migration is
explicit (`migration:<from>:<to>:<steps>` then `ack-migration`) and never runs silently.

## 6. Offline install and uninstall

```sh
# Check a stage without writing anything.
rustage --release deploy-check:dist/stage:/opt/rustage
# Install it (refuses an existing file, an install root, or a project directory).
rustage --release deploy:dist/stage:/opt/rustage
# Remove exactly what the install recorded; projects and settings stay.
rustage --release undeploy:/opt/rustage
# The limits a release note may quote verbatim.
rustage --release boundaries
```

The install root must not contain `project.json`, and no file is overwritten. The install writes
`rustage-install.json`; uninstall reads only that record, so a user project, a settings file or any
other file beside the release is outside its reach by construction.

## 7. The release-candidate journey (T25.05)

```sh
rustage --release freeze:<revision>:<generator> package:<version> widget-revision:<sha> \
                   rc:./target/release/rustage:/tmp/rustage-rc
```

The runner performs `stage → verify → install → uninstall` with the same functions the individual
commands use, then writes a user project and a settings file after installing and checks both
survive the uninstall. It stops at the first failing step and names it.

## 8. Maintenance policy (T25.08)

```sh
rustage --release policy
rustage --release severity:critical
rustage --release drill:dist/stage
rustage --release upgrade:<widget>,<toolchain>,<version>:<widget>,<toolchain>,<version>
```

- `main` takes every fix first; `release/x.y` cherry-picks fixes only; `maintenance/x.y` backports
  blocker and critical fixes with the regression that proved them.
- Blocker and critical defects block a release; major and minor ones do not.
- `drill` re-verifies every staged digest, the language packs and the frozen revision, package and
  widget facts, and reports every failure together.
- `upgrade` answers the regression suites a widget, toolchain or version change triggers.

## 9. Known limitations (as of 2026-11-01)

- **No signing or notarisation command.** `artifact ... signed` records a status; producing the
  signature is the platform tooling's job.
- **No installer packaging.** No `.dmg`, `.msi` or `.deb` builder is in this repository. `--release
  stage` produces the validated input an installer takes, and `deploy` copies it without
  overwriting; registering the result with an operating-system uninstaller is still manual.
- **Signing needs the platform tool and a real identity.** `sign` works with an ad-hoc macOS
  identity for testing; a distributable release needs a Developer ID, a Windows certificate and a
  GPG key respectively. Notarisation needs Apple credentials in a `notarytool` profile.
- **The stage is not a trust boundary.** The digest proves the files are the ones staged; it does
  not establish who built them.
- **No automated RC on other platforms.** `--release rc` is a local runner; Windows, HarmonyOS and
  Linux journeys still need a real machine (xwin/SDK present, not exercised here).
- **`language/` must travel with the binary.** The executable looks beside itself, then in the
  working directory, then `RUSTAGE_LANGUAGE_DIR`; an installer that drops only the binary launches
  with the fallback catalogue.

Verification for this guide: `cargo test --bin rustage app::release` covers freeze, artifacts,
digest verification, SBOM, stage, the update journal, install/uninstall, the maintenance drill and
the candidate journey; the stage and install tests create real directories and check every copied
byte count and digest. The journey was also run against the real debug binary before this guide was
updated.
