#!/usr/bin/env python3
# SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
# SPDX-License-Identifier: MIT
"""Drive the workbench Pages panel with real X11 pointer and keyboard events.

This is the window-level evidence for T13.05/J02: a page is declared, a navigation
edge is declared, the window moves along it, back/forward use their own stacks, and
the closing state refuses a route. The probe never calls the model directly.

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

        def key(name: str, gap: float = 0.08) -> None:
            code = x11.XKeysymToKeycode(display, x11.XStringToKeysym(name.encode()))
            xtst.XTestFakeKeyEvent(display, code, 1, 0)
            x11.XSync(display, 0)
            time.sleep(0.005)
            xtst.XTestFakeKeyEvent(display, code, 0, 0)
            x11.XSync(display, 0)
            time.sleep(gap)

        def type_text(text: str) -> None:
            for character in text:
                key(character, gap=0.05)

        def clear_field() -> None:
            for _ in range(24):
                key("BackSpace")

        # At 1280x800 the left column is 256px wide and holds three 184px slices:
        # Pages (74..258), Outline (266..450), Events (458..642). Inside Pages the
        # field is at y=154..180 and the five buttons at y=184..210.
        first_page = (x + 60, y + 115)
        second_page = (x + 60, y + 133)
        field = (x + 120, y + 167)
        add = (x + 34, y + 197)
        open_button = (x + 82, y + 197)
        back = (x + 130, y + 197)
        forward = (x + 178, y + 197)
        close = (x + 226, y + 197)

        checks: list[tuple[str, bool]] = []

        click(*first_page)
        click(*second_page)
        click(*open_button)
        checks.append((
            "Open follows the declared edge",
            output.wait_for('open "detail"; revision 1', 5.0),
        ))
        click(*back)
        checks.append((
            "Back uses the window's back stack",
            output.wait_for('back -> "home"', 5.0),
        ))
        click(*forward)
        checks.append((
            "Forward uses the window's forward stack",
            output.wait_for('forward -> "detail"', 5.0),
        ))

        click(*field)
        clear_field()
        type_text("settings")
        click(*add)
        checks.append((
            "a new page is declared once",
            output.wait_for('declared page "settings"', 5.0),
        ))
        click(*field)
        clear_field()
        type_text("home->settings")
        click(*add)
        checks.append((
            "a typed from->to declares one navigation",
            output.wait_for('declared navigation "home" -> "settings"', 5.0),
        ))

        click(*close)
        checks.append((
            "Close moves the window to closing",
            output.wait_for("window window closing", 5.0),
        ))
        click(*close)
        checks.append((
            "a second Close finishes the window",
            output.wait_for("window window closed", 5.0),
        ))
        click(*close)
        checks.append((
            "a closed window refuses another close",
            output.wait_for("not closed:", 5.0),
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
