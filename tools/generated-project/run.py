#!/usr/bin/env python3
# SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
# SPDX-License-Identifier: MIT
"""Build and run an independent project from the designer's generated source.

This is the MS07 exit demonstration ("the generated project builds and runs on its own") as a
repeatable harness rather than a claim. It is deliberately not part of `cargo test`: the consumer
needs the pinned widget crate, and rebuilding it inside a test run would both be slow and fight the
test's build lock.

What it does:

1. writes a minimal page document;
2. asks the real binary to generate it with `--generate ... write:<path> run`;
3. writes a standalone consumer crate whose only dependency is the same pinned `rust_widgets`;
4. builds that crate with `cargo build` and runs the produced binary;
5. asserts the program exited 0 and printed its marker.

The generated source is the designer's output, not a fixture in the consumer: replacing the page
changes the source and the build has to follow it.

`--profile mini` and `--profile embedded` ask the generator for its stripped output and build the
consumer with the matching library profile, which is what proves the *generated* source for the
restricted targets links and runs. The host is still the desktop triple; a cross build for the
target device remains a separate piece of evidence.
"""

from __future__ import annotations

import argparse
import re
import shutil
import subprocess
import sys
from pathlib import Path


def pinned_revision(manifest: Path) -> str:
    text = manifest.read_text(encoding="utf-8")
    match = re.search(r'rust_widgets\s*=\s*\{[^}]*rev\s*=\s*"([0-9a-f]+)"', text)
    if not match:
        raise SystemExit(f"no pinned rust_widgets revision in {manifest}")
    return match.group(1)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", default="target/debug/rustage")
    parser.add_argument(
        "--work",
        default="target/generated-project-work",
        help="where the page, generated source and consumer crate are written",
    )
    parser.add_argument("--target-dir", default="target")
    parser.add_argument(
        "--profile",
        choices=["default", "mini", "embedded"],
        default="default",
        help="the target profile the source is generated for",
    )
    parser.add_argument("--timeout", type=int, default=900)
    args = parser.parse_args()

    binary = Path(args.binary)
    if not binary.is_file():
        print(f"{binary} does not exist; run `cargo build --bin rustage`", file=sys.stderr)
        return 1

    work = Path(args.work).resolve()
    if work.exists():
        shutil.rmtree(work)
    (work / "src").mkdir(parents=True)

    page = work / "page.json"
    page.write_text(
        '{"window":{"id":"root","title":"Generated","children":['
        '{"label":{"id":"title","text":"Hello"}},'
        '{"button":{"id":"go","text":"Go"}}]}}',
        encoding="utf-8",
    )

    source = work / "src" / "ui.rs"
    generate = subprocess.run(
        [
            str(binary),
            "--generate",
            f"document:{page}",
            "node:window:2",
            "node:label:0",
            "node:button:0",
            "target:default" if args.profile == "default" else "target:stripped",
            "size:320x200",
            "fn:build_ui",
            f"write:{source}",
            "run",
        ],
        capture_output=True,
        text=True,
        timeout=args.timeout,
    )
    sys.stdout.write(generate.stdout)
    if generate.returncode != 0:
        sys.stderr.write(generate.stderr)
        print(f"generation failed with exit {generate.returncode}", file=sys.stderr)
        return generate.returncode
    if "wrote " not in generate.stdout:
        print("the generator did not report a write", file=sys.stderr)
        return 1
    if not source.is_file() or source.stat().st_size == 0:
        print("the generated source is missing or empty", file=sys.stderr)
        return 1

    rev = pinned_revision(Path("Cargo.toml"))
    features = (
        '["desktop", "runtime-fonts"]'
        if args.profile == "default"
        else f'["{args.profile}"]'
    )
    package = f"generated-consumer-{args.profile}"
    marker = (
        "generated-ui-ok"
        if args.profile == "default"
        else f"generated-{args.profile}-ok"
    )
    (work / "Cargo.toml").write_text(
        f"""[package]
name = "{package}"
version = "0.1.0"
edition = "2024"

[dependencies]
rust_widgets = {{ git = "https://github.com/mikewolfli/rust-widgets", rev = "{rev}", default-features = false, features = {features} }}
""",
        encoding="utf-8",
    )
    (work / "src" / "main.rs").write_text(
        f"""include!("ui.rs");

fn main() {{
    build_ui();
    println!("{marker}");
}}
""",
        encoding="utf-8",
    )

    build = subprocess.run(
        [
            "cargo",
            "build",
            "--manifest-path",
            str(work / "Cargo.toml"),
            "--target-dir",
            args.target_dir,
        ],
        capture_output=True,
        text=True,
        timeout=args.timeout,
    )
    sys.stdout.write(build.stdout)
    if build.returncode != 0:
        sys.stderr.write(build.stderr)
        print(f"the generated project did not build (exit {build.returncode})", file=sys.stderr)
        return build.returncode

    program = Path(args.target_dir) / "debug" / package
    run = subprocess.run([str(program)], capture_output=True, text=True, timeout=60)
    sys.stdout.write(run.stdout)
    if run.returncode != 0:
        sys.stderr.write(run.stderr)
        print(f"the generated program exited {run.returncode}", file=sys.stderr)
        return run.returncode
    if marker not in run.stdout:
        print(f"the generated program did not print {marker!r}", file=sys.stderr)
        return 1

    print(f"generated project ({args.profile}): {source.stat().st_size} bytes of source built and ran")
    print(f"generated project binary: {program}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
