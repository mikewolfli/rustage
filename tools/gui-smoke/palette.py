#!/usr/bin/env python3
# SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
# SPDX-License-Identifier: MIT
"""Drive the workbench command palette with real X11 pointer and keyboard events.

This is the window-level evidence for T05.04/T05.05: the palette is opened from its
toolbar entry, the query is typed into the real line edit, Enter runs the first match,
Escape closes it, and the host log is the fact the assertions wait on. The probe does
not call `dispatch_*` or set widget text directly.

Requirements: `cargo build --features linux-native --bin rustage`, X11, libXtst,
`xwininfo`, `xprop`, and optionally `wmctrl`.
"""

from __future__ import annotations

import ctypes
import os
import re
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from run import Output, client_origin, input_api  # noqa: E402


def workbench_window(pid: int) -> int:
    """The mapped workbench window that belongs to this process."""
    deadline = time.time() + 10.0
    while time.time() < deadline:
        tree = subprocess.run(
            ["xwininfo", "-root", "-tree"], capture_output=True, text=True, check=True
        ).stdout
        for line in tree.splitlines():
            if "design workbench" not in line:
                continue
            match = re.search(r"(0x[0-9a-fA-F]+)", line)
            if not match:
                continue
            wid = int(match.group(1), 16)
            info = subprocess.run(
                ["xwininfo", "-id", hex(wid)], capture_output=True, text=True
            ).stdout
            if not re.search(r"Map State:\s*IsViewable", info):
                continue
            owner = subprocess.run(
                ["xprop", "-id", hex(wid), "_NET_WM_PID"],
                capture_output=True,
                text=True,
            ).stdout
            found = re.search(r"=\s*(\d+)", owner)
            if found and int(found.group(1)) == pid:
                return wid
            if not found:
                continue
        time.sleep(0.1)
    raise SystemExit("no viewable workbench window appeared for this process")


def start_display():
    """Returns `(display, xephyr)`; starts a nested server unless the caller named one."""
    display_name = os.environ.get("PALETTE_DISPLAY")
    if display_name:
        os.environ["DISPLAY"] = display_name
        return display_name, None
    for number in range(90, 110):
        # A lock file means another server (or an aborted run) owns this number. Reusing it
        # would make this probe depend on a display it does not control, and its cleanup would
        # then kill someone else's server.
        if Path(f"/tmp/.X{number}-lock").exists():
            continue
        candidate = f":{number}"
        child = subprocess.Popen(
            ["Xephyr", candidate, "-screen", "1400x900", "-ac", "-noreset"],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            env={**os.environ, "DISPLAY": os.environ.get("DISPLAY", ":0")},
        )
        time.sleep(0.4)
        if child.poll() is not None:
            continue
        if subprocess.run(
            ["xdpyinfo", "-display", candidate], capture_output=True
        ).returncode == 0:
            os.environ["DISPLAY"] = candidate
            return candidate, child
        child.terminate()
    return None, None


def main() -> int:
    binary = Path(sys.argv[1] if len(sys.argv) > 1 else "target/debug/rustage")
    if not binary.is_file():
        print(f"{binary} does not exist; build it with --features linux-native", file=sys.stderr)
        return 2

    # The developer's own session may be locked or otherwise not a reliable input target. A
    # nested X server gives this probe a display it owns, while still exercising the real GTK
    # backend and XTest events. Set PALETTE_DISPLAY to use an already-running display instead.
    display_name, xephyr = start_display()
    if display_name is None:
        print("no nested X server could be started", file=sys.stderr)
        return 2

    process = subprocess.Popen(
        [str(binary), "--workbench-gui", "--exit-after-ms", "18000"],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        bufsize=1,
        env=os.environ.copy(),
    )
    output = Output(process)
    try:
        if not output.wait_for("workbench window", 8.0):
            print(output.text(), file=sys.stderr)
            return 1
        if not output.wait_for("workbench layout", 8.0):
            print(output.text(), file=sys.stderr)
            return 1
        time.sleep(0.3)
        wid = workbench_window(process.pid)
        x, y = client_origin(wid)
        print(f"probe: window {wid:#x} at {x},{y} pid {process.pid} alive {process.poll() is None}", file=sys.stderr)
        x11, xtst = input_api()
        display = x11.XOpenDisplay(display_name.encode())
        if not display:
            print("cannot open the X display", file=sys.stderr)
            return 1
        # A bare X server has no window manager and no other windows to raise above, so
        # XRaiseWindow is unnecessary; it is also the request that fails when GTK has already
        # reparented or recreated the toplevel between the xwininfo read and this call.
        x11.XSetInputFocus(display, wid, 2, 0)
        x11.XSync(display, 0)
        time.sleep(0.5)

        def move(px: int, py: int) -> None:
            xtst.XTestFakeMotionEvent(display, -1, px, py, 0)
            x11.XSync(display, 0)
            time.sleep(0.15)

        def click(px: int, py: int) -> None:
            move(px, py)
            xtst.XTestFakeButtonEvent(display, 1, 1, 0)
            x11.XSync(display, 0)
            time.sleep(0.15)
            xtst.XTestFakeButtonEvent(display, 1, 0, 0)
            x11.XSync(display, 0)
            time.sleep(0.5)

        def key(name: str, gap: float = 0.12) -> None:
            code = x11.XKeysymToKeycode(display, x11.XStringToKeysym(name.encode()))
            xtst.XTestFakeKeyEvent(display, code, 1, 0)
            x11.XSync(display, 0)
            time.sleep(0.05)
            xtst.XTestFakeKeyEvent(display, code, 0, 0)
            x11.XSync(display, 0)
            time.sleep(gap)

        def type_text(text: str) -> None:
            for character in text:
                key(character)

        def open_palette() -> bool:
            # The first toolbar button is "Commands"; its client centre is (61,46).
            for _ in range(3):
                click(x + 61, y + 46)
                if output.wait_for("palette opened", 2.0):
                    return True
            return False

        def focus_after_close() -> int | None:
            text = output.text()
            found = re.findall(r"palette closed; focus Some\((\d+)\)", text)
            return int(found[-1]) if found else None

        checks: list[tuple[str, bool]] = []

        if not open_palette():
            print(output.text(), file=sys.stderr)
            return 1
        opened = re.findall(r"palette opened; previous focus Some\((\d+)\)", output.text())
        previous = int(opened[-1]) if opened else None
        type_text("welcome")
        checks.append((
            "typing filters to view.welcome",
            output.wait_for('palette query "welcome": 1 result(s)', 5.0),
        ))
        # The Run button is the last row of the palette: panel at x=320,y=74, 640x360;
        # its two buttons share (328..952, 458..486). Run's centre is (483,472).
        click(x + 483, y + 472)
        checks.append(("Run executes view.welcome", output.wait_for("palette ran view.welcome", 5.0)))
        checks.append(("Run closes the palette", output.wait_for("palette closed; focus", 5.0)))
        restored = focus_after_close()
        checks.append(("focus returned to the previous control", restored == previous and previous is not None))

        if not open_palette():
            print(output.text(), file=sys.stderr)
            return 1
        type_text("welcome")
        # Close's centre is (797,472).
        click(x + 797, y + 472)
        checks.append(("Close hides the palette", output.wait_for("palette closed; focus", 5.0)))
        checks.append(("Close restores focus", focus_after_close() == previous))

        # A row click selects it; Run then executes the selected command. `save` matches
        # file.save first; the workbench has no file handler, so the observable fact is the
        # resolved command, not a pretend write.
        if not open_palette():
            print(output.text(), file=sys.stderr)
            return 1
        type_text("save")
        click(x + 400, y + 160)
        selected = output.wait_for("palette selected", 3.0)
        click(x + 483, y + 472)
        checks.append(("a row click selects a command", selected))
        checks.append(("Run executes the selected row", output.wait_for("palette ran file.save", 5.0)))
        checks.append(("the palette closed after activation", output.wait_for("palette closed; focus", 5.0)))

        # Enter in the query field runs the selected command; Escape closes without running.
        # Both are real key events through the GTK backend (normalised since 2.8.12).
        if not open_palette():
            print(output.text(), file=sys.stderr)
            return 1
        type_text("welcome")
        key("Return")
        checks.append(("Enter runs the selected command", output.wait_for("palette ran view.welcome", 5.0)))
        checks.append(("Enter closes the palette", output.wait_for("palette closed; focus", 5.0)))

        if not open_palette():
            print(output.text(), file=sys.stderr)
            return 1
        type_text("welcome")
        key("Escape")
        checks.append(("Escape closes the palette", output.wait_for("palette closed; focus", 5.0)))

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
