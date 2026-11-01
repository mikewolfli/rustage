# Extension development

This guide describes the local extension package contract the designer validates today. It is the
documentation half of roadmap task T22.07; the executable example is
[`examples/extension-hello`](../examples/extension-hello), and the compatibility test is
[`the_example_package_still_matches_its_manifest_and_this_host`](../src/app/extension/package.rs).

An extension is **a manifest plus files**. The host does not run arbitrary code from the package: it
validates the manifest, verifies the digest, records where the copy came from, and matches the
package's declared entry points against the open project. Any later execution path is a separate
contract; do not assume a `files/` payload is executed merely because it installed.

## 1. Layout

```text
my-extension/
├── manifest.json
└── files/
    ├── README.md
    └── ...            # any regular files the extension ships
```

- `manifest.json` is required. `files/` is optional but every regular file under it becomes part of
  the package, keyed by its `/`-separated path relative to `files/`.
- A symlink anywhere under `files/` is refused. A package may not reach outside its own directory.
- The directory is installed offline; the recorded source is the directory path. Nothing is fetched.

## 2. Manifest

```json
{
  "id": "com.example.hello",
  "version": "0.1.0",
  "host_min": "0.1.0",
  "host_max": "0.2.0",
  "capabilities": ["widget"],
  "permissions": [],
  "provides": ["hello-card"],
  "license": "MIT",
  "digest": "<sha-256 of the files, see §3>"
}
```

| Field | Meaning | Refused when |
|---|---|---|
| `id` | Stable package identity. | Empty. |
| `version` | `major.minor.patch`. | Not three numeric parts. |
| `host_min` / `host_max` | The host range the extension supports (inclusive). | Reversed, or the running host is outside it. |
| `capabilities` | Capabilities the extension registers. | The host does not publish one of them. Today the host publishes `widget` and `template`. |
| `permissions` | Effects the extension asks to perform. | The host does not publish one of them. Today it publishes `network` and `fs`. |
| `provides` | Entry points a project can reference (control/command names). | Never on its own; an empty list is valid and simply has no project-visible entry point. |
| `license` | SPDX-style licence identifier. | Empty. |
| `digest` | SHA-256 of the files, §3. | It does not match the files. |

A package that fails any check is refused **as a whole**; nothing is half-installed.

## 3. Digest

The digest covers the file **name and contents**, in name order, with length prefixes so that
`"ab" + "c"` cannot collide with `"a" + "bc"`:

```text
SHA-256 over, for each file in sorted name order:
    u64 little-endian byte length of the name
    the name's UTF-8 bytes
    u64 little-endian byte length of the content
    the content bytes
```

The manifest stores the lower-case hex digest. It is an integrity check, not a signature: it proves
the files are the ones the author published. It does not establish trust.

Recompute it after any payload change. The example package's test fails if the manifest and the
payload disagree, which is the intended workflow:

```sh
cargo test --bin rustage app::extension::package::tests::the_example
```

## 4. Install, inspect, pin and roll back

```sh
# Offline install from a directory
rustage --extension install-dir:examples/extension-hello show

# Inline install with the same fields, when the package is built on the command line
rustage --extension id:com.example.hello version:0.1.0 host-min:0.1.0 host-max:0.2.0 \
                   license:MIT capability:widget provides:hello-card \
                   file:README.md:"hello" install

# Inspect the invocation's package
rustage --extension show

# Pin the installed version so an update is refused until it is unpinned
rustage --extension pin:com.example.hello

# Restore the previous package after a successful update
rustage --extension rollback:com.example.hello

# List what the registry holds
rustage --extension list
```

`install` and `install-dir` also run the permission check for the manifest's declared permissions;
a permission the host publishes but the session has not authorized is reported rather than silently
granted. `remove` refuses while a project references one of the extension's `provides` entry points,
and names the dangling names.

## 5. Compatibility testing

A package is compatible with a host when:

1. its files match the digest;
2. the running host version is inside `host_min..=host_max`;
3. every capability and permission it declares is published by the host;
4. every `provides` entry point is unique in the registry.

The example package is a compatibility fixture: it is loaded with `Package::from_directory`, its
digest is checked, and it is validated against the test host. A host-range or capability change that
breaks the example fails the test, so the documentation and the example cannot drift from the code.

## 6. What this contract does not cover

- Executing extension code, sandboxing it, or loading it at runtime inside the designer.
- Remote registries, signing, or author identity.
- Template rendering beyond `Template::render`'s parameter schema (required/optional and kind).

Those are separate roadmap items; this guide does not present them as available.
