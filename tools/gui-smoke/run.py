#!/usr/bin/env python3
# SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
# SPDX-License-Identifier: MIT
"""Run the Linux GUI smoke and assert the real control-event path end to end.

The binary must be built with GTK: `cargo build --features linux-native --bin rustage`.
The script needs an X11 session, `xwininfo`, and libXtst. It drives a real pointer (no
`dispatch_*` calls), so the evidence is GTK's event delivery rather than a test shortcut.

It asserts, in order:

1. a 1280x800 window named `rustage` is mapped;
2. the smoke controls exist and the button region was drawn;
3. one primary click is logged and the worker reports `state=running`;
4. press -> move outside -> release does not add a click;
5. a second primary click reports `state=idle`;
6. the submit button routes `Submit/submit` to its own binding and reports `state=submitted`,
   and a second submit returns to `idle` (multi-control routing, F13);
7. with `--keys`, typing `abc` produces field `change` events and `state=editing`;
8. with `--ime`, an fcitx composition commits `你好` through the library-owned input context.

The keyboard-mode rationale: the pinned rust-widgets revision moves router focus on a pointer
press (`focus_on_pointer_press`) and its GTK window grabs keyboard focus on `map`, so a real
click on the field followed by real keys exercises the library path with no host workaround.
`--ime` is opt-in because it depends on a running input method: the script activates fcitx and
selects pinyin, then restores what it found. The library now owns the toolkit IM context too,
and the host observes the library's pre-edit seam (`WidgetHandle::on_ime_preedit`) to log each
uncommitted frame; the committed `你好` is the observable proof that composition happened
rather than five separate letter commits.

`--ime` is not a claim that every environment composes. If fcitx is inactive or on its
`keyboard-us` method, GTK correctly receives one commit per letter; the script reports that
as a failure of the harness precondition instead of calling it a pass.
"""

from __future__ import annotations

import argparse
import atexit
import ctypes
import os
import re
import shutil
import struct
import subprocess
import sys
import threading
import time

LIMIT = 10.0


class Output:
    """Reads the child's merged output on a thread so the driver can wait for a log fact.

    A fixed sleep is not evidence that a synthetic click landed; the host's own log line is.
    Waiting for it also keeps a lost first click from turning into a misleading click-count
    assertion, and it makes the final output available without a second, blocking read.
    """

    def __init__(self, process: subprocess.Popen[str]) -> None:
        self.process = process
        self.lines: list[str] = []
        self.lock = threading.Lock()
        self.reader = threading.Thread(target=self._read, daemon=True)
        self.reader.start()

    def _read(self) -> None:
        for line in self.process.stdout:
            with self.lock:
                self.lines.append(line)

    def text(self) -> str:
        with self.lock:
            return "".join(self.lines)

    def wait_for(self, needle: str, timeout: float) -> bool:
        deadline = time.time() + timeout
        while time.time() < deadline:
            if needle in self.text():
                return True
            time.sleep(0.05)
        return False


def window_id(pid: int) -> int:
    """The viewable 1280x800 window belonging to *this* process.

    Matching the title alone can select a stale window left by an aborted run; clicking it
    wastes the synthetic events and then fails with `BadWindow`. `_NET_WM_PID` is the identity
    that cannot be confused with another session's window.
    """
    have_xprop = shutil.which("xprop") is not None
    deadline = time.time() + LIMIT
    while time.time() < deadline:
        tree = subprocess.run(
            ["xwininfo", "-root", "-tree"], capture_output=True, text=True, check=True
        ).stdout
        candidates = re.findall(
            r'(0x[0-9a-fA-F]+)\s+"rustage":?\s+\("rustage"\s+"Rustage"\)\s+1280x800',
            tree,
        )
        for candidate in reversed(candidates):
            wid = int(candidate, 16)
            info = subprocess.run(
                ["xwininfo", "-id", hex(wid)], capture_output=True, text=True
            ).stdout
            # A stale window from an earlier session may still be in the tree; only a mapped one
            # can receive the synthetic pointer, and choosing it keeps the harness deterministic.
            if not re.search(r"Map State:\s*IsViewable", info):
                continue
            if not have_xprop:
                return wid
            owner = subprocess.run(
                ["xprop", "-id", hex(wid), "_NET_WM_PID"], capture_output=True, text=True
            ).stdout
            match = re.search(r"=\s*(\d+)", owner)
            if match is None or int(match.group(1)) == pid:
                return wid
        time.sleep(0.1)
    raise SystemExit("no viewable 1280x800 rustage window appeared for this process")


def client_origin(wid: int) -> tuple[int, int]:
    info = subprocess.run(
        ["xwininfo", "-id", hex(wid)], capture_output=True, text=True, check=True
    ).stdout
    x = int(re.search(r"Absolute upper-left X:\s*(-?\d+)", info).group(1))
    y = int(re.search(r"Absolute upper-left Y:\s*(-?\d+)", info).group(1))
    return x, y


def input_api():
    x11 = ctypes.CDLL("libX11.so.6")
    xtst = ctypes.CDLL("libXtst.so.6")
    x11.XOpenDisplay.restype = ctypes.c_void_p
    x11.XOpenDisplay.argtypes = [ctypes.c_char_p]
    x11.XSync.argtypes = [ctypes.c_void_p, ctypes.c_int]
    x11.XCloseDisplay.argtypes = [ctypes.c_void_p]
    x11.XRaiseWindow.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
    x11.XSetInputFocus.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int, ctypes.c_ulong]
    x11.XStringToKeysym.restype = ctypes.c_ulong
    x11.XStringToKeysym.argtypes = [ctypes.c_char_p]
    x11.XKeysymToKeycode.restype = ctypes.c_ubyte
    x11.XKeysymToKeycode.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
    xtst.XTestFakeMotionEvent.argtypes = [
        ctypes.c_void_p,
        ctypes.c_int,
        ctypes.c_int,
        ctypes.c_int,
        ctypes.c_ulong,
    ]
    xtst.XTestFakeButtonEvent.argtypes = [
        ctypes.c_void_p,
        ctypes.c_uint,
        ctypes.c_int,
        ctypes.c_ulong,
    ]
    xtst.XTestFakeKeyEvent.argtypes = [
        ctypes.c_void_p,
        ctypes.c_uint,
        ctypes.c_int,
        ctypes.c_ulong,
    ]
    return x11, xtst


def button_colours(wid: int) -> int:
    """Count distinct pixel values in the button's client rectangle.

    This is a visual lite check, not a baseline diff: a blank clear-colour region has one value,
    while a painted button has a fill, a border and antialiased text. It catches "the window exists
    but the control tree did not draw" without pinning a theme-specific image.
    """
    path = "/tmp/rustage-gui-smoke.xwd"
    subprocess.run(
        ["xwd", "-id", hex(wid), "-silent", "-out", path], check=True
    )
    data = open(path, "rb").read()
    header = struct.unpack(">25I", data[:100])
    header_size, width, bpp, bytes_per_line, ncolors = (
        header[0],
        header[4],
        header[11],
        header[12],
        header[19],
    )
    pos = header_size + ncolors * 12
    stride = bytes_per_line // width
    sample = min(stride, 4)
    colours = set()
    for y in range(40, 80):
        row = pos + y * bytes_per_line
        for x in range(40, 200):
            offset = row + x * stride
            colours.add(data[offset : offset + sample])
    return len(colours)


def fcitx_state() -> str:
    result = subprocess.run(["fcitx5-remote"], capture_output=True, text=True)
    return result.stdout.strip()


def fcitx_im() -> str:
    result = subprocess.run(["fcitx5-remote", "-n"], capture_output=True, text=True)
    return result.stdout.strip()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", default="target/debug/rustage")
    parser.add_argument("--exit-after-ms", type=int, default=10000)
    parser.add_argument(
        "--keys",
        action="store_true",
        help="also type abc into the field and assert the change event",
    )
    parser.add_argument(
        "--ime",
        action="store_true",
        help="also compose nihao with fcitx and assert the committed text 你好",
    )
    args = parser.parse_args()
    if args.ime:
        args.exit_after_ms = max(args.exit_after_ms, 16000)
    elif args.keys:
        args.exit_after_ms = max(args.exit_after_ms, 12000)

    if shutil.which("xwininfo") is None:
        print("xwininfo is required", file=sys.stderr)
        return 1
    if shutil.which("xwd") is None:
        print("xwd is required", file=sys.stderr)
        return 1
    if not os.path.isfile(args.binary):
        print(
            f"{args.binary} does not exist; build it with --features linux-native",
            file=sys.stderr,
        )
        return 1

    if args.ime:
        if shutil.which("fcitx5-remote") is None:
            print("--ime needs fcitx5-remote on PATH", file=sys.stderr)
            return 1
        if not fcitx_state():
            print("--ime needs a running fcitx5 (fcitx5-remote answered nothing)", file=sys.stderr)
            return 1
        initial_state = fcitx_state()
        initial_im = fcitx_im()

        def restore() -> None:
            if initial_im:
                subprocess.run(["fcitx5-remote", "-s", initial_im], capture_output=True)
            if initial_state == "2":
                subprocess.run(["fcitx5-remote", "-o"], capture_output=True)
            else:
                subprocess.run(["fcitx5-remote", "-c"], capture_output=True)

        atexit.register(restore)

    command = [
        args.binary,
        "--launch",
        "--smoke-worker",
        "--exit-after-ms",
        str(args.exit_after_ms),
    ]
    if args.ime:
        command.append("--ime")
    process = subprocess.Popen(
        command,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    )
    atexit.register(lambda: process.poll() is None and process.kill())
    output = Output(process)

    time.sleep(0.6)
    wid = window_id(process.pid)
    # Let the first frame land: a mapped window is not necessarily drawable yet on a loaded X
    # server, and a pointer event delivered before the paint area is realized is lost. Give the
    # window input focus explicitly too: without a window manager's focus handoff the first
    # synthetic click can be delivered to whatever held focus before. The window manager is also
    # asked to activate the window (Metacity/Marco ignores a bare raise for stacking).
    time.sleep(0.6)
    colours = button_colours(wid)
    x, y = client_origin(wid)
    x11, xtst = input_api()
    display = x11.XOpenDisplay(None)
    if not display:
        process.kill()
        raise SystemExit("no X display")
    if shutil.which("wmctrl"):
        # Metacity/Marco can undo a bare raise for stacking. Keeping the smoke window above the
        # overlapping editor window makes synthetic pointer coordinates deterministic without
        # stealing keyboard focus for the mouse-only run.
        subprocess.run(
            ["wmctrl", "-i", "-r", hex(wid), "-b", "add,above"], capture_output=True
        )
    x11.XRaiseWindow(display, wid)
    x11.XSync(display, 0)
    time.sleep(0.5)
    early_failures: list[str] = []
    if not output.wait_for("smoke controls: button", 8.0):
        early_failures.append("the smoke controls never reported themselves")

    def focus_window() -> None:
        """Activates the window only when the mode really needs keyboard input."""
        if shutil.which("wmctrl"):
            subprocess.run(["wmctrl", "-i", "-a", hex(wid)], capture_output=True)
        x11.XRaiseWindow(display, wid)
        x11.XSetInputFocus(display, wid, 2, 0)
        x11.XSync(display, 0)
        time.sleep(0.3)

    def release_keyboard() -> None:
        """Stops later real keystrokes from landing in the smoke field."""
        x11.XSetInputFocus(display, 0, 1, 0)
        x11.XSync(display, 0)

    def move(px: int, py: int) -> None:
        xtst.XTestFakeMotionEvent(display, -1, px, py, 0)
        x11.XSync(display, 0)
        time.sleep(0.25)

    def click(px: int, py: int) -> None:
        # The window may have been restacked by another application between attempts.
        x11.XRaiseWindow(display, wid)
        x11.XSync(display, 0)
        move(px, py)
        xtst.XTestFakeButtonEvent(display, 1, 1, 0)
        x11.XSync(display, 0)
        time.sleep(0.2)
        xtst.XTestFakeButtonEvent(display, 1, 0, 0)
        x11.XSync(display, 0)
        time.sleep(0.6)

    def key(name: str, hold: float = 0.15, gap: float = 0.25) -> None:
        code = x11.XKeysymToKeycode(display, x11.XStringToKeysym(name.encode()))
        xtst.XTestFakeKeyEvent(display, code, 1, 0)
        x11.XSync(display, 0)
        time.sleep(hold)
        xtst.XTestFakeKeyEvent(display, code, 0, 0)
        x11.XSync(display, 0)
        time.sleep(gap)

    def chord(modifier: str, name: str) -> None:
        mod = x11.XKeysymToKeycode(display, x11.XStringToKeysym(modifier.encode()))
        code = x11.XKeysymToKeycode(display, x11.XStringToKeysym(name.encode()))
        xtst.XTestFakeKeyEvent(display, mod, 1, 0)
        x11.XSync(display, 0)
        time.sleep(0.05)
        xtst.XTestFakeKeyEvent(display, code, 1, 0)
        x11.XSync(display, 0)
        time.sleep(0.1)
        xtst.XTestFakeKeyEvent(display, code, 0, 0)
        x11.XSync(display, 0)
        time.sleep(0.05)
        xtst.XTestFakeKeyEvent(display, mod, 0, 0)
        x11.XSync(display, 0)
        time.sleep(0.2)

    def clear_field() -> None:
        """Starts the keystroke assertions from an empty field, not from stray input."""
        chord("Control_L", "a")
        key("BackSpace", hold=0.08, gap=0.2)

    def click_until(px: int, py: int, needle: str, attempts: int = 3) -> bool:
        """Clicks until the host logs the fact, not until a fixed sleep expires."""
        for _ in range(attempts):
            click(px, py)
            if output.wait_for(needle, 2.0):
                return True
        return False

    # Primary button client rect is (40,40)-(200,80); click its centre.
    bx, by = x + 120, y + 60
    if not click_until(bx, by, "smoke button clicked (1)"):
        early_failures.append("the first primary click never appeared in the log")
    # Press on the button, drag off it, release: must not add a click.
    move(bx, by)
    xtst.XTestFakeButtonEvent(display, 1, 1, 0)
    x11.XSync(display, 0)
    time.sleep(0.2)
    move(x + 40, y + 200)
    xtst.XTestFakeButtonEvent(display, 1, 0, 0)
    x11.XSync(display, 0)
    time.sleep(0.9)
    if not click_until(bx, by, "smoke button clicked (2)"):
        early_failures.append("the second primary click never appeared in the log")
    # Submit button client rect is (220,40)-(380,80). It has its own declared event and binding.
    sx, sy = x + 300, y + 60
    click(sx, sy)
    output.wait_for("worker after submit: state=submitted", 3.0)
    click(sx, sy)
    output.wait_for("worker after submit: state=idle", 3.0)

    if args.keys:
        focus_window()
        # The pinned revision moves router focus on a pointer press, and that is the path under
        # test: click the field (client rect (40,100)-(340,132)) rather than focusing it from the
        # host. Without this click the keys would go to whichever control was focused last.
        click(x + 190, y + 116)
        clear_field()
        for name in ("a", "b", "c"):
            key(name)
        release_keyboard()

    if args.ime:
        focus_window()
        # Same pointer-press path as the key smoke; the composition target is the focused field.
        click(x + 190, y + 116)
        clear_field()
        # Make the precondition explicit: an inactive fcitx or its keyboard-us method would make
        # every letter a separate commit, which is correct input-method behaviour and not a
        # composition. The script restores the previous state on exit.
        subprocess.run(["fcitx5-remote", "-o"], capture_output=True)
        subprocess.run(["fcitx5-remote", "-s", "pinyin"], capture_output=True)
        time.sleep(0.5)
        for name in ("n", "i", "h", "a", "o"):
            key(name, hold=0.12, gap=0.28)
        time.sleep(0.8)
        key("space", hold=0.12, gap=0.3)
        time.sleep(1.2)
        release_keyboard()

    try:
        process.wait(timeout=args.exit_after_ms / 1000 + LIMIT)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait()
    output.reader.join(timeout=1.0)
    text = output.text()
    x11.XCloseDisplay(display)

    expected = [
        "smoke controls: button",
        "smoke field text watcher installed",
        "event accepted: Primary/click -> node 1: command count (command)",
        "smoke button clicked (1)",
        "worker after click: state=running",
        "smoke button clicked (2)",
        "worker after click: state=idle",
        "event accepted: Submit/submit (text=\"\") -> node 3: call submit (call)",
        "worker after submit: state=submitted",
        "worker after submit: state=idle",
        "smoke 2 click(s)",
    ]
    if args.keys:
        expected.extend(
            [
                "smoke field changed: abc",
                "event accepted: Field/change",
                "worker after change: state=editing",
            ]
        )
    if args.ime:
        expected.extend(
            [
                "smoke IME: the pinned library owns the toolkit input context",
                "smoke IME preedit: ",
                'event accepted: Field/change (text="你好") -> node 2: call edit (call)',
                "worker after change: state=editing",
            ]
        )
    if args.keys and args.ime:
        expected.append('smoke field final: "abc你好"')
    elif args.keys:
        expected.append('smoke field final: "abc"')
    elif args.ime:
        expected.append('smoke field final: "你好"')
    missing = [line for line in expected if line not in text]
    failures = list(early_failures)
    if colours < 4:
        failures.append(
            f"the button region has {colours} distinct pixel value(s); the control tree did not draw"
        )
    if missing:
        failures.append(f"missing log lines: {missing}")
    if process.returncode != 0:
        failures.append(f"exit code {process.returncode}, expected 0")
    clicks = [int(n) for n in re.findall(r"smoke button clicked \((\d+)\)", text)]
    if clicks != [1, 2]:
        failures.append(f"click sequence {clicks}, expected [1, 2] (the drag must not click)")
    if "event refused" in text:
        failures.append("a real control event was refused by the event model")
    if failures:
        print("\n".join(failures), file=sys.stderr)
        print(text, file=sys.stderr)
        return 1
    print("gui-smoke: click, drag-cancel, multi-control routing and worker states verified")
    if args.keys:
        print("gui-smoke: keyboard change events verified")
    if args.ime:
        print("gui-smoke: real fcitx composition committed 你好")
    print(f"gui-smoke: button region draws {colours} distinct pixel values")
    print(text)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
