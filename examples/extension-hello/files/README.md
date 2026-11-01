# Hello extension

This is a complete, host-validated extension package. It is small on purpose: the point is the
contract, not the payload.

* `manifest.json` states the host range, the capability (`widget`), the entry point it provides
  (`hello-card`) and the SHA-256 of every file under `files/`.
* `files/README.md` is the package payload. Any regular file under `files/` is part of the package;
  a symlink is refused.
* The digest is not a signature. It proves the files are the ones the author published, and
  `rustage --extension install-dir:` refuses the whole package when it disagrees.

Install and inspect it from the repository root:

```sh
rustage --extension install-dir:examples/extension-hello show
rustage --extension list
```

The test at `src/app/extension/package.rs` loads this directory and proves the manifest still
matches the files, so editing one without the other is a failing test rather than a stale example.
