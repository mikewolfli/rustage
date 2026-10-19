#!/usr/bin/env python3
# SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
# SPDX-License-Identifier: MIT
"""A small Debug Adapter Protocol server used to exercise rustage's client.

This is **not** a debugger. It answers the DAP requests the debug client sends with fixed bodies,
so the transport, the session order (`initialize` -> `launch` -> `initialized` ->
`setBreakpoints` -> `configurationDone` -> `launch` response) and the inspection path can be tested
without a platform debugger. The real adapter evidence is separate and is blocked on a machine whose
Developer Mode is off; this file never claims to be that evidence.
"""

from __future__ import annotations

import json
import sys


def read_message():
    length = None
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            return None
        line = line.strip()
        if not line:
            break
        name, _, value = line.partition(b":")
        if name.strip().lower() == b"content-length":
            length = int(value.strip())
    if length is None:
        return None
    return json.loads(sys.stdin.buffer.read(length))


def send(payload):
    body = json.dumps(payload).encode()
    sys.stdout.buffer.write(b"Content-Length: %d\r\n\r\n" % len(body) + body)
    sys.stdout.buffer.flush()


def response(request, body=None, success=True, message=None):
    payload = {
        "seq": request["seq"],
        "type": "response",
        "request_seq": request["seq"],
        "command": request["command"],
        "success": success,
    }
    if body is not None:
        payload["body"] = body
    if message is not None:
        payload["message"] = message
    send(payload)


def event(name, body=None):
    payload = {"seq": 0, "type": "event", "event": name}
    if body is not None:
        payload["body"] = body
    send(payload)


def main():
    source = "/tmp/rustage-dap-stub/main.rs"
    while True:
        request = read_message()
        if request is None:
            return 0
        command = request.get("command")
        arguments = request.get("arguments") or {}
        if command == "initialize":
            response(
                request,
                {
                    "supportsConfigurationDoneRequest": True,
                    "supportsConditionalBreakpoints": True,
                    "supportsHitConditionalBreakpoints": True,
                    "supportsLogPoints": True,
                    "supportsEvaluateForHovers": True,
                    "supportsSetVariable": True,
                },
            )
        elif command == "launch":
            event("initialized")
            response(request)
        elif command == "attach":
            event("initialized")
            response(request)
        elif command == "setBreakpoints":
            points = arguments.get("breakpoints") or []
            body = {
                "breakpoints": [
                    {
                        "verified": True,
                        "line": point.get("line", 0),
                        "source": {"path": source},
                    }
                    for point in points
                ]
            }
            response(request, body)
        elif command == "configurationDone":
            response(request)
            event(
                "stopped",
                {"reason": "breakpoint", "threadId": 1, "line": 7, "allThreadsStopped": True},
            )
        elif command in ("continue", "next", "stepIn", "stepOut", "pause"):
            response(request, {"allThreadsContinued": True})
            if command == "continue":
                event("exited", {"exitCode": 0})
        elif command == "threads":
            response(request, {"threads": [{"id": 1, "name": "main"}]})
        elif command == "stackTrace":
            response(
                request,
                {
                    "totalFrames": 2,
                    "stackFrames": [
                        {
                            "id": 1,
                            "name": "tick",
                            "source": {"path": source},
                            "line": 7,
                            "column": 5,
                        },
                        {
                            "id": 2,
                            "name": "main",
                            "source": {"path": source},
                            "line": 3,
                            "column": 5,
                        },
                    ],
                },
            )
        elif command == "scopes":
            response(
                request,
                {
                    "scopes": [
                        {
                            "name": "Locals",
                            "variablesReference": 100,
                            "expensive": False,
                            "namedVariables": 1,
                        }
                    ]
                },
            )
        elif command == "variables":
            response(
                request,
                {
                    "variables": [
                        {
                            "name": "n",
                            "value": "41",
                            "type": "i32",
                            "variablesReference": 0,
                        }
                    ]
                },
            )
        elif command == "evaluate":
            response(request, {"result": "42", "type": "i32", "variablesReference": 0})
        elif command == "disconnect":
            terminate = arguments.get("terminateDebuggee", True)
            print(f"stub-disconnect terminate={terminate}", file=sys.stderr, flush=True)
            response(request)
            return 0
        else:
            response(request, success=False, message=f"unsupported: {command}")


if __name__ == "__main__":
    raise SystemExit(main())
