#!/usr/bin/env python3
"""JSONL test peer: a read stays pending until turn/interrupt arrives."""
import json
import os
import sys


def send(value):
    print(json.dumps(value), flush=True)


pending = None
for line in sys.stdin:
    request = json.loads(line)
    method = request.get("method")
    if "id" not in request:
        continue
    params = request.get("params", {})
    if method == "initialize":
        result = dict(userAgent="test", codexHome=os.environ["CODEX_HOME"],
                      platformFamily="unix", platformOs="linux")
    elif method == "thread/read":
        thread = params["threadId"]
        if thread == "busy-thread":
            pending = request["id"]
            send(dict(method="test/readBlocked", params={}))
            continue
        if thread == "missing-thread":
            send(dict(id=request["id"], error=dict(code=-32602, message="Thread not found")))
            continue
        cwd = os.environ["TEST_WORKSPACE"]
        if thread == "other-workspace":
            cwd = os.path.dirname(cwd)
        result = dict(thread=dict(id=thread, cwd=cwd))
    elif method == "turn/interrupt":
        if params != dict(threadId="thread-1", turnId="turn-1"):
            raise AssertionError("unexpected interrupt target")
        result = {}
        if pending is not None:
            send(dict(id=pending, result=dict(thread=dict(id="busy-thread"))))
            pending = None
    else:
        send(dict(id=request["id"], error=dict(code=-32601, message="Unexpected method")))
        continue
    send(dict(id=request["id"], result=result))
