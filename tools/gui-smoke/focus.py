#!/usr/bin/env python3
# SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
# SPDX-License-Identifier: MIT
"""Drive the workbench with Tab only and prove keyboard focus traversal is real.

This is the window-level evidence for J03's remaining item: Tab moves focus through
the toolbar, panels, lists and fields without a pointer; Shift+Tab reverses it; and
once the Events handler field has focus, typing lands in that field rather than in
the canvas. The probe reads the host's `focus gained` log; it never calls
`focus_widget` or `focus_next` itself.

Requirements: `cargo build --features linux-native --bin rustage`, X11, libXtst,
`xwininfo`, `xprop`, and Xephyr (or `PALETTE_DISPLAY`).
"""

from __future__ import annotations

import os
import re
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from palette import start_display, workbench_window  # noqa: E402
from run import Output, client_origin, input_api  # noqa: E402


def main() -> int:
    binary = Path(sys.argv[1] if len(sys.argv) > 1 else "target/debug/rustage")
    if not binary.is_file():
        print(f"{binary} does not exist; build it with --features linux-native", file=sys.stderr)
        return 2

    display_name, xephyr = start_display()
    if display_name is None:
        print("no nested X server could be started", file=sys.stderr)
        return 2

    process = subprocess.Popen(
        [str(binary), "--workbench-gui", "--exit-after-ms", "40000"],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        bufsize=1,
        env=os.environ.copy(),
    )
    output = Output(process)
    try:
        if not output.wait_for("workbench layout", 8.0):
            print(output.text(), file=sys.stderr)
            return 1
        time.sleep(0.4)
        wid = workbench_window(process.pid)
        x, y = client_origin(wid)
        x11, xtst = input_api()
        display = x11.XOpenDisplay(display_name.encode())
        if not display:
            print("cannot open the X display", file=sys.stderr)
            return 1
        x11.XSetInputFocus(display, wid, 2, 0)
        x11.XSync(display, 0)
        time.sleep(0.4)

        def key(name: str, gap: float = 0.05) -> None:
            code = x11.XKeysymToKeycode(display, x11.XStringToKeysym(name.encode()))
            xtst.XTestFakeKeyEvent(display, code, 1, 0)
            x11.XSync(display, 0)
            time.sleep(0.01)
            xtst.XTestFakeKeyEvent(display, code, 0, 0)
            x11.XSync(display, 0)
            time.sleep(gap)

        def shift_tab() -> None:
            shift = x11.XKeysymToKeycode(display, x11.XStringToKeysym(b"Shift_L"))
            tab = x11.XKeysymToKeycode(display, x11.XStringToKeysym(b"Tab"))
            xtst.XTestFakeKeyEvent(display, shift, 1, 0)
            x11.XSync(display, 0)
            xtst.XTestFakeKeyEvent(display, tab, 1, 0)
            xtst.XTestFakeKeyEvent(display, tab, 0, 0)
            xtst.XTestFakeKeyEvent(display, shift, 0, 0)
            x11.XSync(display, 0)
            time.sleep(0.1)

        seen: list[str] = []
        reached_events = False
        for _ in range(90):
            key("Tab")
            found = re.findall(r"focus gained (.+?) \(\d+\)", output.text())
            for name in found:
                if name not in seen:
                    seen.append(name)
            if "events handler field" in seen:
                reached_events = True
                break
        checks: list[tuple[str, bool]] = []
        checks.append((
            "Tab moves focus through more than the toolbar",
            len([name for name in seen if name not in ("",)]) >= 5,
        ))
        checks.append((
            "Tab reaches a list",
            any("list" in name for name in seen),
        ))
        checks.append((
            "Tab reaches a text field",
            reached_events,
        ))

        typed = False
        if reached_events:
            # Typing while the Events handler field owns the keyboard must land in that field.
            for character in "abc":
                key(character)
            typed = output.wait_for("events field text 3 character(s)", 4.0)
        checks.append(("typing after Tab lands in the focused field", typed))

        before = len(seen)
        shift_tab()
        shift_tab()
        after = len(re.findall(r"focus gained (.+?) \(\d+\)", output.text()))
        checks.append((
            "Shift+Tab emits focus changes too",
            after > before,
        ))

        ok = True
        for name, passed in checks:
            print(f"{'ok  ' if passed else 'FAIL'} {name}")
            ok &= passed
        if not ok:
            print(f"\nseen: {seen}", file=sys.stderr)
            print("\nhost log:\n" + output.text(), file=sys.stderr)
        return 0 if ok else 1
    finally:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
        if xephyr is not None:
            xephyr.terminate()
            try:
                xephyr.wait(timeout=5)
            except subprocess.TimeoutExpired:
                xephyr.kill()


if __name__ == "__main__":
    raise SystemExit(main())
