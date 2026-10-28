#!/usr/bin/env python3
# SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
# SPDX-License-Identifier: MIT
"""Drive the workbench object tree and design canvas with real X11 events.

This is the window-level evidence for T09.02–T09.04: the Outline is the tree view of the
same scene, a canvas click selects the topmost node under the pointer, a hidden or locked
node is refused with the reason that applies, and a drag in empty canvas selects every
selectable node it crosses. Every assertion waits on a host log line produced by the real
pointer path; the harness never dispatches an event or sets widget text directly.

Requirements: `cargo build --features linux-native --bin rustage`, X11, libXtst,
`xwininfo`, `xprop`, `xwd` and Pillow (the screenshot is evidence, not a gate).
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

# Canvas geometry at 1280x800. The design space is 600x400, uniformly scaled into the
# panel and centred: scale = min(688/600, 532/400), origin = (264, 146).
CANVAS_ORIGIN = (264, 146)
CANVAS_SCALE = 688 / 600


def design(x: int, y: int) -> tuple[int, int]:
    """A design-space point in client coordinates."""
    return (
        CANVAS_ORIGIN[0] + round(x * CANVAS_SCALE),
        CANVAS_ORIGIN[1] + round(y * CANVAS_SCALE),
    )


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
        if not output.wait_for("scene ready", 8.0):
            print(output.text(), file=sys.stderr)
            return 1
        time.sleep(0.4)
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

        def move(px: int, py: int, gap: float = 0.08) -> None:
            xtst.XTestFakeMotionEvent(display, -1, px, py, 0)
            x11.XSync(display, 0)
            time.sleep(gap)

        def click(px: int, py: int, gap: float = 0.4) -> None:
            move(px, py, 0.12)
            xtst.XTestFakeButtonEvent(display, 1, 1, 0)
            x11.XSync(display, 0)
            time.sleep(0.08)
            xtst.XTestFakeButtonEvent(display, 1, 0, 0)
            x11.XSync(display, 0)
            time.sleep(gap)

        def drag(points: list[tuple[int, int]], gap: float = 0.06) -> None:
            move(*points[0], gap=0.12)
            xtst.XTestFakeButtonEvent(display, 1, 1, 0)
            x11.XSync(display, 0)
            time.sleep(0.1)
            for point in points[1:]:
                move(*point, gap=gap)
            xtst.XTestFakeButtonEvent(display, 1, 0, 0)
            x11.XSync(display, 0)
            time.sleep(0.5)

        def canvas(px: int, py: int) -> tuple[int, int]:
            dx, dy = design(px, py)
            return x + dx, y + dy

        # Outline rows are 20px tall and start at y=114: window, Demo, button, label,
        # hidden note, footer.
        def row(index: int) -> tuple[int, int]:
            return (x + 100, y + 114 + index * 20)

        checks: list[tuple[str, bool]] = []
        checks.append((
            "the scene starts with six nodes and the Demo panel selected",
            "scene ready: 6 node(s), selection [1]" in output.text(),
        ))

        # A hidden row is refused with the hidden reason, not the locked one.
        click(*row(4))
        checks.append((
            "a hidden row is refused and names hiding",
            output.wait_for(
                "refused tree selection of node 4: node 4 is hidden", 4.0
            ),
        ))
        # A locked row is drawn in the tree but is not selectable.
        click(*row(5))
        checks.append((
            "a locked row is refused and names locking",
            output.wait_for(
                "refused tree selection of node 5: node 5 is locked", 4.0
            ),
        ))

        # The label is painted after the button and overlaps it: a click in the overlap
        # must select the label, which is the topmost node there.
        click(*canvas(300, 190))
        checks.append((
            "a canvas click selects the topmost overlapping node (label)",
            output.wait_for("selected node 3 (label) from canvas", 4.0),
        ))
        checks.append((
            "the Inspector retargets at the selected node's text contract",
            output.wait_for("inspector rows: text: agreed: label", 4.0),
        ))

        # Outside the label but inside the button.
        click(*canvas(110, 130))
        checks.append((
            "a canvas click under the label selects the button",
            output.wait_for("selected node 2 (button) from canvas", 4.0),
        ))

        # A tree click selects the same node through the other view.
        click(*row(3))
        checks.append((
            "a tree row selects the node the canvas would",
            output.wait_for("selected node 3 (label) from tree", 4.0),
        ))

        # A drag through empty canvas and both widgets selects every selectable node it
        # crosses: window, Demo, button and label (the hidden note is skipped).
        drag([canvas(20, 20), canvas(180, 120), canvas(350, 250)])
        checks.append((
            "a marquee selects every selectable node it crosses",
            output.wait_for("marquee ", 4.0) and "4 node(s)" in output.text(),
        ))
        checks.append((
            "the marquee names the selected nodes",
            "0 window, 1 Demo, 2 button, 3 label" in output.text(),
        ))

        time.sleep(0.6)
        subprocess.run(
            ["xwd", "-display", display_name, "-id", hex(wid), "-out", "/tmp/scene-after.xwd"],
            capture_output=True,
        )
        subprocess.run(
            "xwdtopnm /tmp/scene-after.xwd > /tmp/scene-after.ppm",
            shell=True,
            capture_output=True,
        )
        try:
            from PIL import Image

            Image.open("/tmp/scene-after.ppm").convert("RGB").save("target/scene-after.png")
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
