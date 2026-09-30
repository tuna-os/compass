#!/usr/bin/env python3
"""Scripted stand-in for compass-input-server, for the engine's tests.

Reads u32-prefixed (little-endian) JSON-RPC frames from stdin, appends each
request's method and params as one JSON line to $FAKE_INPUT_LOG, and answers
as the real server does: getCapabilities with {"injection": true},
createSnippet with {"ok": false}, removeSnippet with {"removed": false}, the
rest with null. Exits 0 when stdin closes. Never touches a device.
"""

import json
import os
import struct
import sys


def read_frame(stream):
    header = stream.read(4)
    if len(header) < 4:
        return None
    (length,) = struct.unpack("<I", header)
    payload = stream.read(length)
    if len(payload) < length:
        return None
    return json.loads(payload)


def write_frame(stream, message):
    payload = json.dumps(message).encode()
    stream.write(struct.pack("<I", len(payload)) + payload)
    stream.flush()


RESULTS = {
    "Snippet/getCapabilities": {"injection": True},
    "Snippet/createSnippet": {"ok": False},
    "Snippet/removeSnippet": {"removed": False},
}


def main():
    log = os.environ.get("FAKE_INPUT_LOG")
    stdin = sys.stdin.buffer
    stdout = sys.stdout.buffer
    while True:
        request = read_frame(stdin)
        if request is None:
            return 0
        if log:
            with open(log, "a") as f:
                f.write(json.dumps({"method": request["method"], "params": request.get("params")}) + "\n")
        write_frame(stdout, {
            "id": request["id"],
            "jsonrpc": "2.0",
            "result": RESULTS.get(request["method"]),
        })


if __name__ == "__main__":
    sys.exit(main())
