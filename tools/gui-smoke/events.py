#!/usr/bin/env python3
# SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
# SPDX-License-Identifier: MIT
"""Drive the workbench Events panel with real X11 pointer and keyboard events.

This is the window-level evidence for T13.03/T13.04: a declared event's row is
selected, a handler is typed and bound, the same event is then triggered, and
unbinding removes the one attachment. The probe never calls the model or
dispatches an event directly; every assertion waits on the host log.

Requirements: `cargo build --features linux-native --bin rustage`, X11, libXtst,
`xwininfo`, `xprop`, and Xephyr (or `PALETTE_DISPLAY`).
"""

from __future__ import annotations

import os
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

        def key(name: str, gap: float = 0.15) -> None:
            code = x11.XKeysymToKeycode(display, x11.XStringToKeysym(name.encode()))
            xtst.XTestFakeKeyEvent(display, code, 1, 0)
            x11.XSync(display, 0)
            time.sleep(0.005)
            xtst.XTestFakeKeyEvent(display, code, 0, 0)
            x11.XSync(display, 0)
            time.sleep(gap)

        def key_text(text: str) -> None:
            for character in text:
                key(character, gap=0.12)

        # Events panel at 1280x800: (0,358) 260x280. Rows are 20 px high and start at
        # y=388, so the button's `pressed` row (index 3 in the library's declaration order)
        # is centred near y+464. The handler field is at y=542..568, the Bind/Unbind/Fire row
        # at y=572..598.
        pressed_event = (x + 130, y + 464)
        field = (x + 130, y + 555)
        bind = (x + 48, y + 585)
        unbind = (x + 126, y + 585)
        fire = (x + 204, y + 585)

        checks: list[tuple[str, bool]] = []

        click(*pressed_event)
        checks.append((
            "selecting a declared event row is reported",
            output.wait_for("events row 3 -> pressed", 4.0),
        ))
        click(*field)
        key_text("count")
        click(*bind)
        checks.append((
            "a typed handler binds to the selected node",
            output.wait_for("bind pressed -> count on node 1: revision 1", 5.0),
        ))
        click(*fire)
        checks.append((
            "a trigger on the wired node reports the handler",
            output.wait_for("fired pressed:", 5.0),
        ))
        checks.append((
            "the trigger says a handler is wired",
            "a handler is wired" in output.text(),
        ))
        click(*unbind)
        checks.append((
            "unbinding removes the one attachment",
            output.wait_for("unbind pressed on node 1: revision 2", 5.0),
        ))
        click(*fire)
        checks.append((
            "a trigger after unbinding is unwired, not a delivery",
            output.wait_for("no handler is wired", 5.0),
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
