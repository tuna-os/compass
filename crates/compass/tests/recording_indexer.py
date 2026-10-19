#!/usr/bin/env python3
"""Record the real engine's configure/start/stop traffic in a temporary log.

A protocol-only helper: it never scans directories or opens an index. Queries
return a synthetic row under each configured root so the live shared search
client is exercised as well as its outgoing configuration.
"""

import json
import os
import sys

from fake_indexer import read_frame, reply


def record(**fields):
    with open(os.environ["COMPASS_TEST_INDEXER_LOG"], "a", encoding="utf-8") as log:
        log.write(json.dumps({"pid": os.getpid(), **fields}) + "\n")


def main():
    record(event="start")
    config = {"paths": [], "excluded_paths": []}
    while True:
        msg = read_frame(sys.stdin.buffer)
        if msg is None:
            record(event="stop")
            return
        method = msg.get("method", "")
        record(method=method, params=msg.get("params", {}))
        if method == "FileIndexer/configure":
            config = msg["params"]["config"]
            reply(sys.stdout.buffer, msg["id"], None)
        elif method == "FileIndexer/query":
            reply(sys.stdout.buffer, msg["id"], {
                "matches": [
                    {
                        "path": os.path.join(root, "probe.txt"),
                        "rank": 1.0,
                        "category": "Document",
                        "mime_type": "text/plain",
                    }
                    for root in config["paths"]
                ],
            })
        elif method == "FileIndexer/rebuildIndex":
            reply(sys.stdout.buffer, msg["id"], None)


if __name__ == "__main__":
    main()
