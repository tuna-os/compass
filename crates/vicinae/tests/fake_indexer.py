#!/usr/bin/env python3
"""Scripted stand-in for vicinae-file-indexer, for the client tests.

Reads u32-prefixed JSON-RPC frames from stdin and answers them:
- FileIndexer/configure and FileIndexer/rebuildIndex get null results.
- FileIndexer/query gets one canned match echoing the request text.
- Every started scan emits a Started event; queries also emit a Succeeded
  event for entrypoint /tmp/fake, so the client tracks and forgets scans.

Usage: fake_indexer.py <mode>, where mode is:
- serve: run until stdin closes (the polite shutdown path).
- crash: answer one query, then exit(1) with no further output.
- crash-now: answer the configure, then exit(1) with scans still tracked.
- exit-zero: exit(0) immediately, for the clean-exit path.
"""

import json
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


def write_frame(stream, payload):
    data = json.dumps(payload).encode()
    stream.write(struct.pack("<I", len(data)))
    stream.write(data)
    stream.flush()


def reply(stream, msg_id, result):
    write_frame(stream, {"jsonrpc": "2.0", "id": msg_id, "result": result})


def event(stream, name, params):
    write_frame(stream, {"jsonrpc": "2.0", "method": name, "params": params})


SCAN_ID = 41


def handle(stream, msg):
    method = msg.get("method", "")
    msg_id = msg.get("id")
    if method == "FileIndexer/configure":
        reply(stream, msg_id, None)
        event(
            stream,
            "FileIndexer/scanStatusChanged",
            {
                "status": {
                    "scan_id": SCAN_ID,
                    "kind": "Full",
                    "state": "Started",
                    "entrypoint": "/tmp/fake",
                    "processed_file_count": 0,
                }
            },
        )
    elif method == "FileIndexer/rebuildIndex":
        reply(stream, msg_id, None)
    elif method == "FileIndexer/query":
        req = msg.get("params", {}).get("req", {})
        reply(
            stream,
            msg_id,
            {
                "matches": [
                    {
                        "path": "/tmp/fake/" + req.get("text", ""),
                        "rank": 1.5,
                        "category": "Document",
                        "mime_type": "text/plain",
                    }
                ]
            },
        )
        event(
            stream,
            "FileIndexer/scanStatusChanged",
            {
                "status": {
                    "scan_id": SCAN_ID,
                    "kind": "Incremental",
                    "state": "Succeeded",
                    "entrypoint": "/tmp/fake",
                    "processed_file_count": 7,
                }
            },
        )
    # Unknown methods stay silent, like the real route table.


def main():
    mode = sys.argv[1] if len(sys.argv) > 1 else "serve"
    stream = sys.stdin.buffer
    if mode == "exit-zero":
        return 0
    while True:
        msg = read_frame(stream)
        if msg is None:
            return 0
        handle(sys.stdout.buffer, msg)
        sys.stdout.buffer.flush()
        if mode == "crash" and msg.get("method") == "FileIndexer/query":
            return 1
        if mode == "crash-now" and msg.get("method") == "FileIndexer/configure":
            return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
