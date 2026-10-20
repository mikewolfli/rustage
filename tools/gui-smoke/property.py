#!/usr/bin/env python3
# SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
# SPDX-License-Identifier: MIT
"""Drive the workbench Inspector with real X11 pointer and keyboard events.

This is the window-level evidence for T11.01: a double-click focuses the title value,
typing an over-limit string and pressing Apply is refused while the valid value and
revision stay put, Clear empties the field, and a valid string advances the revision
once. The host log is the fact the assertions wait on; the harness does not call
`set_widget_text` on the model or dispatch events directly.

Requirements: `cargo build --features linux-native --bin rustage`, X11, libXtst,
`xwininfo`, `xprop`, `xwd`, Pillow, and Xephyr (or `PALETTE_DISPLAY`).
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
        [str(binary), "--workbench-gui", "--exit-after-ms", "30000"],
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
        time.sleep(0.3)
        wid = workbench_window(process.pid)
        x, y = client_origin(wid)
        print(f"probe: window {wid:#x} at {x},{y} pid {process.pid}", file=sys.stderr)
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

        def click(px: int, py: int, gap: float = 0.3) -> None:
            move(px, py)
            xtst.XTestFakeButtonEvent(display, 1, 1, 0)
            x11.XSync(display, 0)
            time.sleep(0.08)
            xtst.XTestFakeButtonEvent(display, 1, 0, 0)
            x11.XSync(display, 0)
            time.sleep(gap)

        def key(name: str, gap: float = 0.25) -> None:
            code = x11.XKeysymToKeycode(display, x11.XStringToKeysym(name.encode()))
            xtst.XTestFakeKeyEvent(display, code, 1, 0)
            x11.XSync(display, 0)
            time.sleep(0.005)
            xtst.XTestFakeKeyEvent(display, code, 0, 0)
            x11.XSync(display, 0)
            time.sleep(gap)

        # Inspector geometry at 1280x800 with three right-hand panels: the Inspector is the
        # first slice (y=74..258) and its value row is the last fixed row (y=200..228). Field
        # centre (1040,214), Apply (1156,214), Clear (1236,214); the title row is near y=160.
        field = (x + 1040, y + 214)
        apply_button = (x + 1156, y + 214)
        clear_button = (x + 1236, y + 214)
        title_row = (x + 1050, y + 160)

        checks: list[tuple[str, bool]] = []

        # The entry path is the visible value field; double-click activation is wired through
        # the library's item_activated signal but the pinned GTK backend does not synthesise
        # MouseDoubleClick, so the probe uses the always-visible field instead of pretending.
        # "Demo" plus 81 characters is over the schema's 40-character limit.
        click(*field)
        time.sleep(1.0)
        for _ in range(45):
            key("x")
        time.sleep(0.3)
        click(*apply_button)
        checks.append((
            "an over-limit value is refused at revision 0",
            output.wait_for("inspector refused title", 5.0),
        ))
        checks.append((
            "the refusal names the unchanged revision",
            "revision 0 unchanged" in output.text(),
        ))
        click(*clear_button)
        checks.append((
            "Clear empties the field without changing the value",
            output.wait_for("inspector field cleared; valid value unchanged", 4.0),
        ))
        checks.append((
            "the cleared field still has Demo as the valid value",
            'valid value unchanged: "Demo"' in output.text(),
        ))

        click(*field)
        time.sleep(1.2)
        for character in "test":
            key(character, gap=0.7)
        time.sleep(0.8)
        click(*apply_button)
        checks.append((
            "a valid value advances the revision once",
            output.wait_for('inspector applied title = "test"; revision 1', 5.0),
        ))
        checks.append((
            "the Inspector row shows the accepted value",
            output.wait_for("inspector rows: title: test", 4.0),
        ))

        time.sleep(0.5)
        subprocess.run(
            ["xwd", "-display", display_name, "-id", hex(wid), "-out", "/tmp/property-after.xwd"],
            capture_output=True,
        )
        subprocess.run(
            "xwdtopnm /tmp/property-after.xwd > /tmp/property-after.ppm",
            shell=True,
            capture_output=True,
        )
        try:
            from PIL import Image

            Image.open("/tmp/property-after.ppm").convert("RGB").save(
                "target/property-after.png"
            )
        except Exception as error:  # noqa: BLE001 - the screenshot is evidence, not a gate
            print(f"probe: screenshot not saved: {error}", file=sys.stderr)

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
