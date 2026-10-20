#!/usr/bin/env python3
# SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
# SPDX-License-Identifier: MIT
"""Drive the workbench Values inspector and Data panel with real X11 events.

This is the window-level evidence for T14.05: the Values button reads the
design/computed/running layers with their versions while the effect counter stays
at zero, and a Data edit advances the version once. A refused value keeps the
model and the version where they were. The probe never calls `trigger`.

Requirements: `cargo build --features linux-native --bin rustage`, X11, libXtst,
`xwininfo`, `xprop`, and Xephyr (or `PALETTE_DISPLAY`).
"""

from __future__ import annotations

import os
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

    process = subprocess.Popen(  # noqa: F821 - imported below for the probe's own clarity
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

        def move(px: int, py: int) -> None:
            xtst.XTestFakeMotionEvent(display, -1, px, py, 0)
            x11.XSync(display, 0)
            time.sleep(0.1)

        def click(px: int, py: int, gap: float = 0.35) -> None:
            move(px, py)
            xtst.XTestFakeButtonEvent(display, 1, 1, 0)
            x11.XSync(display, 0)
            time.sleep(0.08)
            xtst.XTestFakeButtonEvent(display, 1, 0, 0)
            x11.XSync(display, 0)
            time.sleep(gap)

        def key(name: str, gap: float = 0.12) -> None:
            code = x11.XKeysymToKeycode(display, x11.XStringToKeysym(name.encode()))
            xtst.XTestFakeKeyEvent(display, code, 1, 0)
            x11.XSync(display, 0)
            time.sleep(0.005)
            xtst.XTestFakeKeyEvent(display, code, 0, 0)
            x11.XSync(display, 0)
            time.sleep(gap)

        # Inspector at 1280x800 is the first right-hand slice (74..258): its value row is
        # y=154..180 and the theme row y=184..210. Values centre (1227,167), Theme (1236,197).
        values = (x + 1227, y + 167)
        theme_field = (x + 1080, y + 197)
        theme_button = (x + 1236, y + 197)
        # Data panel is the second right-hand slice (266..450): rows start at y=298 and the
        # field/Apply row is y=376..402.
        data_count_row = (x + 130, y + 325)
        data_field = (x + 1080, y + 389)
        data_apply = (x + 1236, y + 389)

        checks: list[tuple[str, bool]] = []

        click(*values)
        checks.append((
            "Values reads the selected node",
            output.wait_for("inspect rows:", 5.0),
        ))
        checks.append((
            "the design layer is shown with its version",
            output.wait_for("design: Demo (v 1)", 5.0),
        ))
        checks.append((
            "the running layer is unavailable rather than copied",
            "runtime: not available" in output.text(),
        ))
        checks.append((
            "the read-only inspection dispatched no business",
            "effects 0" in output.text(),
        ))

        click(*data_count_row)
        time.sleep(0.3)
        click(*data_field)
        for _ in range(6):
            key("BackSpace")
        key("7")
        click(*data_apply)
        checks.append((
            "a typed sample value advances the data version",
            output.wait_for("data applied count = \"7\"; version 2", 5.0),
        ))
        checks.append((
            "the applied edit still dispatched no business",
            "effects 0" in output.text(),
        ))

        click(*data_field)
        for _ in range(6):
            key("BackSpace")
        key("x")
        click(*data_apply)
        checks.append((
            "a non-integer value is refused",
            output.wait_for("data refused count", 5.0),
        ))
        checks.append((
            "the refusal names the unchanged version",
            "not applied at version 2" in output.text(),
        ))

        click(*theme_field)
        for character in "dark":
            key(character, gap=0.12)
        click(*theme_button)
        checks.append((
            "a theme expands one token across the selection",
            output.wait_for("theme applied: theme dark: var(--accent) #ff9f0a", 5.0),
        ))
        click(*values)
        checks.append((
            "the computed layer shows the expanded theme value",
            output.wait_for("computed: #ff9f0a (v 3)", 5.0),
        ))
        checks.append((
            "the design layer keeps the token reference",
            "design: var(--accent) (v 3)" in output.text(),
        ))
        checks.append((
            "the theme review dispatched no business",
            "effects 0" in output.text(),
        ))

        ok = True
        for name, passed in checks:
            print(f"{'ok  ' if passed else 'FAIL'} {name}")
            ok &= passed
        if not ok:
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
