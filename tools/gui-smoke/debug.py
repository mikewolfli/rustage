#!/usr/bin/env python3
# SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
# SPDX-License-Identifier: MIT
"""Drive the workbench debug panel through a real DAP session.

The panel's Start button spawns a worker that compiles a debuggee with the host's own
`rustc`, starts the real adapter (gdb's DAP), binds a breakpoint, stops, and reads
threads/stack/locals. The probe clicks the panel's real buttons and waits on the host log;
it never calls the debug model or dispatches an event directly.

Requirements: `cargo build --features linux-native --bin rustage`, X11, libXtst,
`xwininfo`, `xprop`, `rustc`, and an installed DAP adapter (gdb 15.1 on this host).
"""

from __future__ import annotations

import atexit
import ctypes
import os
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from palette import start_display, workbench_window  # noqa: E402
from run import Output, client_origin, input_api  # noqa: E402

# The right column is 320px wide; with three panels the Debug panel is the third
# slice (y=458..642). Its action row is y=606..634, four buttons across x=968..1272.
START = (1004, 620)
CONTINUE = (1081, 620)
STEP = (1158, 620)
STOP = (1242, 620)


def main() -> int:
    binary = Path(sys.argv[1] if len(sys.argv) > 1 else "target/debug/rustage")
    if not binary.is_file():
        print(f"{binary} does not exist; build it with --features linux-native", file=sys.stderr)
        return 2

    display_name, xephyr = start_display()
    if display_name is None:
        print("no nested X server could be started", file=sys.stderr)
        return 2
    print(
        f"probe: display {display_name} xephyr {xephyr.pid if xephyr else 'external'}",
        file=sys.stderr,
    )

    process = subprocess.Popen(
        [str(binary), "--workbench-gui", "--exit-after-ms", "60000"],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        bufsize=1,
        env=os.environ.copy(),
        start_new_session=True,
    )
    output = Output(process)

    # The Xlib I/O-error handler calls C `exit(1)` when the display disappears, which bypasses
    # Python's `finally`. A C-level atexit handler still runs, so a crashed nested server cannot
    # leave the workbench window (or the Xephyr it owns) behind.
    def cleanup() -> None:
        try:
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=2)
                except subprocess.TimeoutExpired:
                    process.kill()
        except Exception:  # noqa: BLE001 - cleanup must not raise on the way out
            pass
        try:
            if xephyr is not None and xephyr.poll() is None:
                xephyr.terminate()
                try:
                    xephyr.wait(timeout=2)
                except subprocess.TimeoutExpired:
                    xephyr.kill()
        except Exception:  # noqa: BLE001
            pass

    atexit.register(cleanup)
    _cleanup_cb = ctypes.CFUNCTYPE(None)(cleanup)
    try:
        ctypes.CDLL(None).atexit(_cleanup_cb)
    except Exception:  # noqa: BLE001 - the Python atexit still covers a normal return
        pass
    try:
        if not output.wait_for("workbench layout", 8.0):
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

        def click(point: tuple[int, int], gap: float = 0.5) -> None:
            px, py = x + point[0], y + point[1]
            xtst.XTestFakeMotionEvent(display, -1, px, py, 0)
            x11.XSync(display, 0)
            time.sleep(0.15)
            xtst.XTestFakeButtonEvent(display, 1, 1, 0)
            x11.XSync(display, 0)
            time.sleep(0.08)
            xtst.XTestFakeButtonEvent(display, 1, 0, 0)
            x11.XSync(display, 0)
            time.sleep(gap)

        def count_wait(needle: str, before: int, timeout: float) -> bool:
            deadline = time.time() + timeout
            while time.time() < deadline:
                if output.text().count(needle) > before:
                    return True
                time.sleep(0.1)
            return False

        def clicked(point: tuple[int, int], needle: str, timeout: float = 3.0) -> bool:
            """Clicks until the host log records the command; XTest can drop one press."""
            before = output.text().count(needle)
            for _ in range(3):
                click(point)
                if count_wait(needle, before, timeout):
                    return True
            return False

        checks: list[tuple[str, bool]] = []

        checks.append((
            "Start is accepted by the panel",
            clicked(START, "debug start requested (accepted true)", 4.0),
        ))
        checks.append((
            "the real adapter stops at the bound breakpoint",
            output.wait_for("debug: stopped: breakpoint at line", 45.0),
        ))
        checks.append((
            "the breakpoint is bound because the adapter verified it",
            "debug row: breakpoint 1 at " in output.text()
            and "bound at " in output.text(),
        ))
        checks.append((
            "the bound breakpoint maps to the design node",
            "-> node 1" in output.text(),
        ))
        checks.append((
            "a thread is reported",
            "debug row: thread 1" in output.text(),
        ))
        checks.append((
            "the top frame carries the adapter's source position",
            "debug row: frame 0" in output.text()
            and "node_1" in output.text()
            and " at " in output.text(),
        ))
        checks.append((
            "a local variable is read from the stopped frame",
            "debug row: variable n: i32 = 41" in output.text(),
        ))
        checks.append((
            "the source position maps back to the selected node",
            "-> node 1" in output.text()
            and "unmapped: no generated source map" not in output.text(),
        ))

        # The worker's rows are mirrored on the next main-loop turn; let that frame paint before
        # capturing it, otherwise `xwd` shows the pre-update surface.
        output.wait_for("debug rows shown", 3.0)
        time.sleep(0.4)
        shot = subprocess.run(
            ["xwd", "-display", display_name, "-id", hex(wid), "-out", "/tmp/debug-after.xwd"],
            capture_output=True,
        )
        if shot.returncode != 0:
            print(f"probe: xwd failed: {shot.stderr!r}", file=sys.stderr)
        subprocess.run(
            "xwdtopnm /tmp/debug-after.xwd > /tmp/debug-after.ppm",
            shell=True,
            capture_output=True,
        )
        try:
            from PIL import Image

            Image.open("/tmp/debug-after.ppm").convert("RGB").save("target/debug-after.png")
        except Exception as error:  # noqa: BLE001 - the screenshot is evidence, not a gate
            print(f"probe: screenshot not saved: {error}", file=sys.stderr)

        checks.append((
            "Step is accepted while stopped",
            clicked(STEP, "debug step over requested (accepted true)", 4.0),
        ))
        checks.append((
            "Step advances the stopped line",
            output.wait_for("at line 4 (thread 1)", 10.0),
        ))

        checks.append((
            "Run is accepted while stopped",
            clicked(CONTINUE, "debug continue requested (accepted true)", 4.0),
        ))
        checks.append((
            "Run runs the program to its exit",
            output.wait_for("debug: exited", 10.0),
        ))

        stops_before = output.text().count("debug: stopped: breakpoint at line 3")
        checks.append((
            "a second session starts after the first exited",
            clicked(START, "debug start requested (accepted true)", 4.0)
            and count_wait("debug: stopped: breakpoint at line 3", stops_before, 45.0),
        ))
        checks.append((
            "Stop is accepted while stopped",
            clicked(STOP, "debug stop requested (accepted true)", 4.0),
        ))
        checks.append((
            "Stop ends the session",
            output.wait_for("debug: session ended", 10.0),
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
