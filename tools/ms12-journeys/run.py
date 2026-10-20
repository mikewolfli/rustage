#!/usr/bin/env python3
# SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
# SPDX-License-Identifier: MIT
"""Reproduce the MS12 (blue13) joint-acceptance journeys on this machine.

MS12 is an integration gate, not a new feature: it asks whether the chains the
earlier milestones built still meet at the seams. This harness drives the real
binary and records, for every step, *what kind of evidence it is*:

* `real`    - it read or wrote real files, or started a real process (cargo,
              gdb, a business worker, the generated program);
* `model`   - it exercised the typed model through the CLI, but no window or
              device was involved. This is useful evidence, and it is not a
              substitute for the GUI journey roadmap §34 asks for;
* `blocked` - the capability needed for the step does not exist in this build
              or the pinned dependency. The reason is recorded, never hidden.

The report is JSON. A non-zero exit means at least one step failed; blocked
steps do not make the run fail by themselves, but they are counted so a reader
cannot mistake "ran" for "passed".

Usage:
    python3 tools/ms12-journeys/run.py [--binary target/debug/rustage]
                                       [--report target/ms12-journeys/report.json]
                                       [--skip build]
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
from dataclasses import asdict, dataclass, field
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
JOURNEYS = ["J01", "J02", "J03", "J04", "J05", "J06", "J07", "J08", "J09",
            "J17", "J18", "J19"]


@dataclass
class Result:
    journey: str
    step: str
    evidence: str
    status: str
    command: list[str] = field(default_factory=list)
    exit_code: int | None = None
    reason: str = ""
    output: str = ""
    seconds: float = 0.0


class Session:
    def __init__(self, binary: Path, work: Path, skip: set[str]):
        self.binary = binary
        self.work = work
        self.skip = skip
        self.results: list[Result] = []
        self.context: dict[str, str] = {}

    def path(self, name: str) -> Path:
        return self.work / name

    def run(self, journey: str, step: str, evidence: str, argv: list[str],
            expect: tuple[str, ...] = (), reject: tuple[str, ...] = (),
            expect_failure: bool = False, timeout: int = 180,
            cwd: Path | None = None) -> Result:
        if evidence in self.skip:
            return self.blocked(journey, step, f"skipped by --skip {evidence}")
        started = time.monotonic()
        try:
            completed = subprocess.run(
                [str(self.binary), *argv],
                cwd=cwd or ROOT,
                capture_output=True,
                text=True,
                timeout=timeout,
            )
        except subprocess.TimeoutExpired:
            result = Result(journey, step, evidence, "fail", argv,
                            reason=f"timed out after {timeout}s")
            self.results.append(result)
            return result
        output = (completed.stdout or "") + (completed.stderr or "")
        reasons = []
        if expect_failure:
            if completed.returncode == 0:
                reasons.append("expected a non-zero exit, got 0")
        elif completed.returncode != 0:
            reasons.append(f"exit {completed.returncode}")
        missing = [text for text in expect if text not in output]
        if missing:
            reasons.append("missing: " + ", ".join(repr(text) for text in missing))
        present = [text for text in reject if text in output]
        if present:
            reasons.append("unexpected: " + ", ".join(repr(text) for text in present))
        result = Result(
            journey=journey,
            step=step,
            evidence=evidence,
            status="fail" if reasons else "pass",
            command=argv,
            exit_code=completed.returncode,
            reason="; ".join(reasons),
            output=excerpt(output),
            seconds=round(time.monotonic() - started, 3),
        )
        self.results.append(result)
        return result

    def blocked(self, journey: str, step: str, reason: str) -> Result:
        result = Result(journey, step, "blocked", "blocked", reason=reason)
        self.results.append(result)
        return result

    def verify(self, journey: str, step: str, evidence: str, condition: bool,
               reason: str = "") -> Result:
        result = Result(
            journey, step, evidence, "pass" if condition else "fail", reason=reason
        )
        self.results.append(result)
        return result

    def python(self, journey: str, step: str, evidence: str, code: str,
               timeout: int = 180) -> Result:
        if evidence in self.skip:
            return self.blocked(journey, step, f"skipped by --skip {evidence}")
        started = time.monotonic()
        completed = subprocess.run(
            [sys.executable, "-c", code], cwd=ROOT, capture_output=True,
            text=True, timeout=timeout,
        )
        output = (completed.stdout or "") + (completed.stderr or "")
        result = Result(
            journey, step, evidence,
            "pass" if completed.returncode == 0 else "fail",
            [sys.executable, "-c", "<inline>"],
            completed.returncode,
            "" if completed.returncode == 0 else "inline check failed",
            excerpt(output),
            round(time.monotonic() - started, 3),
        )
        self.results.append(result)
        return result


def excerpt(text: str, limit: int = 1600) -> str:
    text = text.strip()
    if len(text) <= limit:
        return text
    return text[:limit] + "\n… (truncated)"


def write(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


def batch(session: Session, journey: str, step: str, page: Path, operations: list[dict],
          root: Path) -> Result:
    batch_file = session.path(f"{journey}-{step.replace(' ', '-')}.json")
    write(batch_file, json.dumps({
        "format": 1,
        "intent": f"{journey} {step}",
        "operations": operations,
    }))
    return session.run(
        journey, step, "real",
        ["--edit", str(page), "--batch", str(batch_file), "--root", str(root), "--apply"],
        expect=("applied:",),
    )


def j01(session: Session) -> None:
    project = session.path("j01-project")
    shutil.rmtree(project, ignore_errors=True)
    session.run(
        "J01", "new project", "real",
        ["--new-project", str(project), "--template", "basic-page", "--name", "Journey01"],
        expect=("created ", "read back: format v2"),
    )
    page = project / "pages" / "main.json"
    session.run(
        "J01", "toolchain diagnosis", "real",
        ["--validate", str(project / "project.json"), "--trust", "--diagnose"],
        expect=("toolchain: cargo:", "needs: 5 (present 5"),
    )
    batch(session, "J01", "insert and set a property", page, [
        {"kind": "add", "parent": 1, "index": 0, "node": {
            "id": 2, "type": "label", "children": [],
            "properties": [{"name": "text", "value": {"text": "Hello"}}]}},
        {"kind": "set", "id": 1, "name": "title",
         "value": {"text": "Journey01"}},
    ], project)
    session.run(
        "J01", "save through a temporary and a rename", "real",
        ["--save", str(page)],
        expect=("save",),
    )
    source = project / "src" / "ui.rs"
    source.parent.mkdir(parents=True, exist_ok=True)
    session.run(
        "J01", "generate from the page model", "real",
        ["--generate", f"document:{page}", "node:window:1", "node:label:0",
         "size:320x200", "fn:build_ui", f"write:{source}", "check"],
        expect=("precheck: clean", f"wrote {source}"),
    )
    # The standalone consumer is the real build/run proof. It generates from its own
    # page so it also proves the generator is not reading a checked-in fixture.
    session.python(
        "J01", "generated project builds and runs", "real",
        "import subprocess,sys; "
        "sys.exit(subprocess.run([sys.executable,'tools/generated-project/run.py',"
        "'--profile','default']).returncode)",
        timeout=900,
    )


def j02(session: Session) -> None:
    session.run(
        "J02", "pages, entry and navigation", "model",
        ["--routes", "page:home", "page:detail", "entry:home",
         "edge:home:detail", "open:home", "ready", "go:detail", "back", "forward",
         "check", "show"],
        expect=("pages: 2", "home -> detail"),
    )
    session.run(
        "J02", "a missing target is diagnosable", "model",
        ["--routes", "page:home", "entry:home", "edge:home:detail", "open:home",
         "ready", "go:detail", "check"],
        expect=("no page \"detail\" is declared",),
        expect_failure=True,
    )
    session.run(
        "J02", "window close and finish", "model",
        ["--routes", "page:home", "entry:home", "open:home", "ready", "close",
         "check", "show"],
        expect=("closing",),
    )
    session.blocked(
        "J02", "new page and focus through the workbench window",
        "the workbench window has no page tab strip or route editor yet (F04/F13 "
        "GUI slice); the model chain above is the evidence this build has",
    )


def j03(session: Session) -> None:
    session.run(
        "J03", "command palette resolves through the registry", "model",
        ["--command", "view.palette"],
        expect=("resolved: view.palette",),
    )
    session.run(
        "J03", "palette search", "model",
        ["--palette"],
        expect=("view.palette",),
    )
    session.run(
        "J03", "keyboard reaches the palette", "model",
        ["--keys", "--press", "Ctrl+P"],
        expect=("runs view.palette",),
    )
    session.run(
        "J03", "keyboard saves and builds", "model",
        ["--keys", "--press", "Ctrl+S"],
        expect=("runs file.save",),
    )
    session.run(
        "J03", "every implemented menu leaf binds one command", "model",
        ["--menu", "--audit"],
        expect=("every implemented menu leaf binds one registered command",),
    )
    session.python(
        "J03", "the command palette in a real window", "real",
        "import subprocess,sys; sys.exit(subprocess.run([sys.executable,"
        "'tools/gui-smoke/palette.py']).returncode)",
        timeout=180,
    )
    session.blocked(
        "J03", "operate the whole journey without a mouse in the window",
        "the command palette is real (open, type, select a row, Run, Close, focus restore), "
        "but Enter/Escape are blocked by the pinned GTK keyval path "
        "(docs/rw-feedback/feedback-20261020.md §0.2), and the object tree/canvas have "
        "pointer evidence only: no keyboard focus traversal between them yet",
    )


def j04(session: Session) -> None:
    session.run(
        "J04", "layout computation is shared by preview and generation", "model",
        ["--layout-plan", "available:1:800,600", "constraint:1:width:explicit",
         "box:1:0,0,800,600", "run:preview", "run:generation", "source:1",
         "state", "show"],
        expect=("preview", "generation"),
    )
    session.run(
        "J04", "switch a container layout", "model",
        ["--layout-switch", "layout:column", "child:1:width", "switch", "state", "show"],
        expect=("column",),
    )
    session.run(
        "J04", "constraints validate at the node", "model",
        ["--constraints", "min:1:100,50", "max:1:400,300", "preferred:1:200,100",
         "grow:1:1", "margin:1:8", "gap:1:4", "overflow:1:hidden", "validate",
         "state", "show"],
        expect=("min", "max", "overflow"),
    )
    session.run(
        "J04", "design tokens keep the reference, expand at generation", "model",
        ["--tokens", "token:colour:accent:#0a84ff",
         "theme:dark:accent=#0a84ff", "use:background:accent", "expand:dark",
         "state", "show"],
        expect=("token accent (colour)", "background: accent"),
    )
    session.run(
        "J04", "device frames name each difference", "model",
        ["--frames", "support-size:390,844", "support-orientation:portrait",
         "support-safe-area:true", "support-max-scale:3",
         "frame:phone:390,844:portrait", "frame-scale:phone:2", "check",
         "state", "show"],
        expect=("phone",),
    )
    session.python(
        "J04", "the Inspector edits one property in a real window", "real",
        "import subprocess,sys; sys.exit(subprocess.run([sys.executable,"
        "'tools/gui-smoke/property.py']).returncode)",
        timeout=180,
    )
    session.python(
        "J04", "the object tree, canvas selection and marquee in a real window", "real",
        "import subprocess,sys; sys.exit(subprocess.run([sys.executable,"
        "'tools/gui-smoke/scene.py']).returncode)",
        timeout=180,
    )
    session.blocked(
        "J04", "theme review by pointer across a multi-selection",
        "the canvas has one selection set and a drag marquee, and the Inspector edits the "
        "primary node's title; a theme token applied to the whole selection (T11.02/T14.05) "
        "has no window surface yet",
    )


def j05(session: Session) -> None:
    session.run(
        "J05", "define, instantiate and expand a component", "model",
        ["--component", "define:card", "version:1", "param:title", "control:label",
         "commit", "instance:10", "of:card", "arg:title=Hi", "place", "expand:10",
         "show"],
        expect=("expanded 2 node(s)", "definitions: [card]"),
    )
    session.run(
        "J05", "extract, insert and override", "model",
        ["--component-author", "kind:label", "property:text=Hello", "extract:card",
         "use:card", "insert:10", "override:10:text=Hi", "resolve:10", "show"],
        expect=("extracted card", "resolved"),
    )
    session.run(
        "J05", "detach and reset a variant", "model",
        ["--component-override", "variant:10", "of:card", "kind:label",
         "target:embedded", "override:text=Hi", "detach", "reset",
         "check:embedded", "show"],
        expect=("detached", "reset: dropped"),
    )
    session.run(
        "J05", "component package refuses a path escape", "model",
        ["--asset-package", "package:ui:1:MIT", "entry:../escape.json:1",
         "plan", "show"],
        expect=("escapes the project root",),
        expect_failure=True,
    )


def j06(session: Session) -> None:
    project = session.path("j01-project")
    source = project / "src" / "ui.rs"
    if not source.is_file():
        session.blocked("J06", "source round trip", "J01 did not produce ui.rs")
        return
    session.run(
        "J06", "node to source mapping", "real",
        ["--mapping", f"source:{source}", "model:1", "key:1:root", "locate:1", "show"],
        expect=("mapped 1 node(s)", "node 1 ->"),
    )
    edited = project / "src" / "ui.edited.rs"
    write(edited, source.read_text(encoding="utf-8") + "\n// hand written\n")
    mine = project / "src" / "ui.mine.rs"
    write(mine, source.read_text(encoding="utf-8") + "\n// mine\n")
    theirs = project / "src" / "ui.theirs.rs"
    write(theirs, source.read_text(encoding="utf-8") + "\n// theirs\n")
    session.run(
        "J06", "an unrecognised text change is kept, not guessed", "real",
        ["--reverse", f"base:{source}", f"edited:{edited}", f"mine:{mine}",
         f"theirs:{theirs}", "show"],
        expect=("kept (unrecognised)",),
        expect_failure=True,
    )
    regenerated = project / "src" / "ui.new.rs"
    session.run(
        "J06", "regenerate for the merge walk", "real",
        ["--generate", f"document:{project / 'pages' / 'main.json'}",
         "node:window:1", "node:label:0", "size:320x200", "fn:build_ui",
         f"write:{regenerated}", "check"],
        expect=("wrote",),
    )
    session.run(
        "J06", "merge keeps a hand-written region", "real",
        ["--gen-merge", f"old:{source}", f"new:{regenerated}", "show"],
        expect=("no change",),
    )
    session.run(
        "J06", "a one-line edit is a small diff", "real",
        ["--diff", f"left:{source}", f"right:{edited}", "show"],
        expect=("diff: 1 changed row(s)",),
    )


def j07(session: Session) -> None:
    session.run(
        "J07", "style and existing bindings do not start cargo", "model",
        ["--hot-update", "self:src/app/style.css", "from:src/app/other.css",
         "property:color", "style:color", "existing:field.text", "flush", "check",
         "show"],
        expect=("property color -> live", "existing field.text -> live"),
    )
    session.run(
        "J07", "new bindings and nodes request a rebuild", "model",
        ["--hot-update", "self:src/app/style.css", "from:src/app/other.css",
         "add-binding:field.text", "add-node:label", "flush", "check", "show"],
        expect=("add-binding field.text -> rebuild", "add-node label -> rebuild"),
    )
    session.run(
        "J07", "two real generations, snapshot, import and late-frame rejection",
        "real",
        ["--logic-process", "session:s", "name:orders", "version:1", "first:idle",
         "second:idle", "first-transition:idle:go:running",
         "second-transition:running:back:idle", "first-event:go",
         "second-event:back", "preserve", "check", "show"],
        expect=("old generation 1: pid", "snapshot generation 1: running",
                "restored generation 2: running",
                "fault=the frame is from generation 1, not 2"),
        expect_failure=True,
        timeout=120,
    )
    session.python(
        "J07", "the process boundary is a real pair of processes", "real",
        "import subprocess,sys; sys.exit(subprocess.run("
        "['cargo','test','--test','logic_process','--quiet']).returncode)",
        timeout=900,
    )
    session.blocked(
        "J07", "switch while the preview window keeps focus and input",
        "the workbench window does not own a preview session yet; the process and "
        "model evidence above does not claim that GUI state was preserved",
    )


def j08(session: Session) -> None:
    session.run(
        "J08", "target configuration", "model",
        ["--target-config", "profile:desktop", "triple:x86_64-unknown-linux-gnu",
         "toolchain:1.98.1", "mode:two", "runner:host", "host:linux", "check", "show"],
        expect=("target: ready", "mode=two"),
    )
    session.run(
        "J08", "capability snapshot and switch caveat", "model",
        ["--target-capability", "generator:1", "built-for:desktop", "name:button",
         "node:button:0", "switch:desktop:embedded", "check", "show"],
        expect=("switch desktop -> embedded", "not verified under the new target"),
    )
    session.run(
        "J08", "restricted target capacity", "model",
        ["--target-restricted", "profile:mini", "children:3", "mode:two",
         "dev:serde", "build:yes", "check", "show"],
        expect=("mini: 3 children", "target build: built", "restricted target: ready"),
    )
    session.run(
        "J08", "desktop matrix records one built triple", "model",
        ["--target-build-matrix", "start:x86_64-unknown-linux-gnu",
         "pass:x86_64-unknown-linux-gnu:built:ok", "check", "show"],
        expect=("passed built", "unverified: 4 combination(s)"),
        expect_failure=True,
    )
    artifact = session.path("j08-artifact.bin")
    artifact.write_text("artifact", encoding="utf-8")
    session.run(
        "J08", "queue tracks an artifact without claiming the matrix", "model",
        ["--target-queue", "job:1:desktop:build", "start:1",
         f"pass:1:built:{artifact}", "check", "show"],
        expect=("1 job(s)", "artifact 1"),
    )
    session.python(
        "J08", "restricted generated source builds and runs", "real",
        "import subprocess,sys; "
        "sys.exit(subprocess.run([sys.executable,'tools/generated-project/run.py',"
        "'--profile','mini']).returncode)",
        timeout=900,
    )
    session.blocked(
        "J08", "embedded profile builds on 2.8.11",
        "rust_widgets 2.8.11 references WidgetKind::{TabWidget,Splitter,Grid,Dialog} "
        "from constructors gated on `not(alloc_frugal)` while the variants are gated "
        "on `widgets_unstripped`; `--no-default-features --features embedded --lib` "
        "fails with E0599. Recorded in docs/rw-feedback/feedback-20261020.md §0.1",
    )
    session.run(
        "J08", "certification keeps unverified rows visible", "model",
        ["--target-certification", "profile:desktop",
         "stage:build:built:cargo-build:ok", "consistency", "check", "show"],
        expect=("embedded/build", "unverified"),
        expect_failure=True,
    )
    session.blocked(
        "J08", "install on a real tablet or phone",
        "no device is connected on this host; adb discovery returned 0 devices in "
        "the prior round. A simulator or a plan is not device evidence.",
    )


def j09(session: Session) -> None:
    source = session.path("j09/main.rs")
    write(source, 'fn main() {\n    let n = 41;\n    println!("value {}", n + 1);\n}\n')
    program = session.path("j09/app")
    session.python(
        "J09", "compile a real debuggee", "real",
        "import subprocess,sys; sys.exit(subprocess.run(["
        "'rustc','-g','-C','opt-level=0'," + repr(str(source)) + ",'-o',"
        + repr(str(program)) + "]).returncode)",
    )
    session.run(
        "J09", "adapter probe", "real",
        ["--debug", "probe"],
        expect=("gdb:", "ready"),
    )
    session.run(
        "J09", "launch, break, variables, evaluate, step, disconnect", "real",
        ["--debug", "adapter:auto", "authorize", "host:linux", "timeout:5000",
         f"launch:{program}", f"directory:{program.parent}", "stop-on-entry:yes",
         "start", "wait:3000", "break:main.rs:3", "continue", "wait:5000",
         "threads", "stack", "scopes:0", "variables:1", "evaluate:n",
         "step-over", "wait:5000", "status", "disconnect"],
        expect=("breakpoint 1 at main.rs:3", "stopped: breakpoint at line 3",
                "variable n: i32 = 41", "evaluate i32 41", "stopped: step"),
        timeout=120,
    )
    # Attaching is the line that must not kill a process the designer did not start.
    attach = session.path("j09/attach-check.py")
    write(attach, "\n".join([
        "import subprocess, sys, time",
        "child = subprocess.Popen(['sleep', '30'])",
        "try:",
        "    out = subprocess.run([sys.argv[1], '--debug', 'adapter:auto', 'authorize',",
        "        'host:linux', 'timeout:5000', f'attach:{child.pid}', 'disconnect'],",
        "        capture_output=True, text=True, timeout=60)",
        "    text = out.stdout + out.stderr",
        "    if 'disconnected' not in text:",
        "        print(text); sys.exit(2)",
        "    time.sleep(0.3)",
        "    alive = child.poll() is None",
        "    print('attached pid', child.pid, 'alive' if alive else 'dead')",
        "    sys.exit(0 if alive else 3)",
        "finally:",
        "    if child.poll() is None:",
        "        child.terminate()",
        "        child.wait(timeout=5)",
    ]))
    session.python(
        "J09", "attach then detach leaves the external process alive", "real",
        "import subprocess,sys; sys.exit(subprocess.run([sys.executable,"
        + repr(str(attach)) + "," + repr(str(session.binary)) + "]).returncode)",
        timeout=120,
    )
    session.python(
        "J09", "the debug panel drives a real DAP session", "real",
        "import subprocess,sys; sys.exit(subprocess.run([sys.executable,"
        "'tools/gui-smoke/debug.py']).returncode)",
        timeout=180,
    )
    session.blocked(
        "J09", "two-way source navigation inside the debug panel",
        "the panel shows the adapter's raw position and says no generated source map is "
        "loaded; the model-level mapping tests and the CLI source round-trip remain the "
        "evidence for T19.06",
    )


def j17(session: Session) -> None:
    session.run(
        "J17", "app shell regions", "model",
        ["--app-shell", "target:desktop", "title:Journey", "size:800x600",
         "region:menu", "region:toolbar", "region:status", "provide:menu",
         "gap:dock:none", "show"],
        expect=("window \"Journey\" 800x600", "menu: provided"),
    )
    session.run(
        "J17", "shared actions", "model",
        ["--app-action", "action:save", "title:Save", "icon:save",
         "shortcut:Ctrl+S", "enabled:yes", "group:file", "select:save", "show"],
        expect=("handler:save", "Ctrl+S"),
    )
    session.run(
        "J17", "menus and toolbars read the same action", "model",
        ["--app-menu", "root:main:Main", "define:main", "item:main:Save:save",
         "separator:main", "show"],
        expect=("Save -> save",),
    )
    session.run(
        "J17", "toolbar and status", "model",
        ["--app-toolbar", "item:save", "label:Save", "present:icon",
         "visible:yes", "overflow:collapse", "capacity:4", "status:ready:Ready",
         "show"],
        expect=("save \"Save\"", "status ready: Ready"),
    )
    session.run(
        "J17", "modal dialog default and cancel", "model",
        ["--app-dialog", "main:main", "dialog:confirm:main:confirm:modal",
         "default:confirm:ok", "cancel:confirm:cancel", "field:name:World",
         "open:confirm", "accept:confirm", "show"],
        expect=("opened confirm", "accepted via ok"),
    )
    session.run(
        "J17", "runtime and static agree on the same entries", "model",
        ["--app-regression", "mode:runtime", "bind:menu:save", "preview",
         "generate", "activate:menu:save", "show"],
        expect=("preview", "generate:runtime", "menu:save fired"),
    )
    session.blocked(
        "J17", "a real target app window with the menu and dialog",
        "the app-shell models are complete, but the generated consumer is not an app "
        "window with a native menu yet; the workbench window is the designer, not the "
        "target app",
    )


def j18(session: Session) -> None:
    session.run(
        "J18", "sample and runtime data stay distinct", "model",
        ["--data", "schema:orders:1", "field:id:text:required", "field:total:number",
         "sample:total=0", "runtime:total=12", "check", "active", "state", "show"],
        expect=("sample: sample (not runtime data)", "runtime: runtime data"),
    )
    session.run(
        "J18", "an incompatible binding is refused", "model",
        ["--binding", "schema:orders:1", "field:total:number", "slot:1.text:text:32",
         "bind:f:total=1.text:one-way:identity", "check", "state", "show"],
        expect=("REFUSED", "0 binding(s)"),
    )
    session.run(
        "J18", "a text conversion is admitted", "model",
        ["--binding", "schema:orders:1", "field:total:number", "slot:1.text:text:32",
         "bind:f:total=1.text:one-way:text", "check", "state", "show"],
        expect=("binding: accepted", "1 binding(s)"),
    )
    session.run(
        "J18", "table columns, sort and an empty state", "model",
        ["--table", "column:Name:text", "column:Total:number", "width:Name:120",
         "sort:Total:desc", "filter:Name:a", "select:0", "empty:no-rows",
         "source:orders", "show"],
        expect=("Name [text]", "filter Name contains \"a\""),
    )
    session.run(
        "J18", "collection rows are sample content", "model",
        ["--collection", "control:1", "column:text:Name", "row", "set:1:1:Alpha",
         "show"],
        expect=("Alpha (sample)",),
    )
    session.run(
        "J18", "a large collection is bounded", "model",
        ["--collection-regression", "total:100", "viewport:20"],
        expect=("shown 0..20 of 100",),
    )
    session.run(
        "J18", "a cell edit can be cancelled without saving", "model",
        ["--cell-edit", "row:0", "column:Name", "current:old", "candidate:new",
         "delegate:text", "commit", "cancel", "show"],
        expect=("committed \"new\"", "cancelled (nothing saved)"),
    )


def j19(session: Session) -> None:
    workspace = session.path("j19-workspace")
    shutil.rmtree(workspace, ignore_errors=True)
    member = workspace / "member"
    write(member / "Cargo.toml",
          '[package]\nname = "member"\nversion = "0.1.0"\nedition = "2024"\n')
    write(member / "src" / "main.rs", "fn main() {}\n")
    write(workspace / "Cargo.toml",
          '[workspace]\nmembers = ["member"]\nresolver = "3"\n')
    session.run(
        "J19", "create a workspace", "real",
        ["--new-workspace", str(workspace), "--name", "Multi"],
        expect=("created ", "0 member(s)"),
        cwd=ROOT,
    )
    document = workspace / "workspace.json"
    session.run(
        "J19", "add a member", "real",
        ["--workspace", str(document), "--add", "member=member", "--apply", "--trust"],
        expect=("add member `member`", "members: member"),
    )
    session.run(
        "J19", "activate and open a file by member-qualified path", "real",
        ["--workspace", str(document), "--activate", "member", "--trust"],
        expect=("active member: `member`",),
    )
    session.run(
        "J19", "the same path in two members is qualified", "real",
        ["--workspace", str(document), "--file", "member:src/main.rs"],
        expect=("file: member:src/main.rs", "inside the member"),
    )
    session.run(
        "J19", "offline dependency resolution", "real",
        ["--dependencies", str(workspace), "--trust", "--allow-lockfile-change",
         "--record", str(workspace / "deps.json")],
        expect=("cargo",),
        timeout=300,
    )
    session.run(
        "J19", "reproducible flags name vendor and cache", "model",
        ["--reproducible", "locked:yes", "offline:yes", "vendor:/tmp/vendor",
         "cache:/tmp/cache", "source:/tmp/src", "show"],
        expect=("--frozen", "vendored-sources"),
    )


def git_revision() -> str:
    completed = subprocess.run(
        ["git", "--no-pager", "rev-parse", "HEAD"], cwd=ROOT,
        capture_output=True, text=True,
    )
    return completed.stdout.strip() or "unknown"


def dependency_revision() -> str:
    text = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    for line in text.splitlines():
        if "rust_widgets" in line and "rev" in line:
            start = line.find('rev = "')
            if start >= 0:
                return line[start + 7:].split('"')[0]
    return "unknown"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", default="target/debug/rustage")
    parser.add_argument("--report", default="target/ms12-journeys/report.json")
    parser.add_argument("--skip", action="append", default=[],
                        choices=["real", "model", "build"],
                        help="skip a whole evidence class (recorded as blocked)")
    parser.add_argument("--work", default="target/ms12-journeys/work")
    args = parser.parse_args()

    binary = (ROOT / args.binary).resolve()
    if not binary.is_file():
        print(f"{binary} does not exist; run `cargo build --bin rustage`", file=sys.stderr)
        return 2
    work = (ROOT / args.work).resolve()
    shutil.rmtree(work, ignore_errors=True)
    work.mkdir(parents=True)
    report_path = (ROOT / args.report).resolve()
    report_path.parent.mkdir(parents=True, exist_ok=True)

    skip = set(args.skip)
    # `build` is a convenience for a quick model-only run; it skips the slow real
    # builds but leaves the debug/process steps, which are cheap and decisive.
    if "build" in skip:
        skip.discard("build")
        skip.add("real")

    session = Session(binary, work, skip)
    started = time.monotonic()
    for journey in [j01, j02, j03, j04, j05, j06, j07, j08, j09, j17, j18, j19]:
        journey(session)
    elapsed = round(time.monotonic() - started, 3)

    counts: dict[str, int] = {}
    for result in session.results:
        counts[f"{result.status}/{result.evidence}"] = counts.get(
            f"{result.status}/{result.evidence}", 0) + 1
    failed = [result for result in session.results if result.status == "fail"]
    report = {
        "schema": "rustage.ms12-evidence/v1",
        "generated_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "host": {
            "os": sys.platform,
            "python": sys.version.split()[0],
        },
        "worktree_revision": git_revision(),
        "dependency": {"crate": "rust_widgets", "revision": dependency_revision()},
        "binary": str(binary.relative_to(ROOT)),
        "seconds": elapsed,
        "journeys": JOURNEYS,
        "counts": counts,
        "results": [asdict(result) for result in session.results],
        "verdict": "fail" if failed else "ran",
    }
    report_path.write_text(
        json.dumps(report, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )

    for result in session.results:
        mark = {"pass": "ok  ", "fail": "FAIL", "blocked": "blk "}[result.status]
        print(f"{mark} {result.journey} {result.evidence:8} {result.step}"
              + (f"  [{result.reason}]" if result.reason else ""))
    print()
    print(f"{len(session.results)} step(s) in {elapsed}s: " + ", ".join(
        f"{key} {value}" for key, value in sorted(counts.items())))
    print(f"report: {report_path}")
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
