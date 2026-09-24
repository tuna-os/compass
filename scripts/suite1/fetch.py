#!/usr/bin/env python3
"""Install Suite 1's corpus, and write the harness's plan for it.

    scripts/suite1/fetch.py <extensions-dir> <plan.json> [--only raycast|vicinae]

Downloads every extension corpus.json names from its store — the prebuilt
bundles the stores serve, exactly what a user's install gets — and unpacks
each where the engine looks: `<extensions-dir>/store.raycast.<name>` and
`store.vicinae.<name>`, the ids the C++ and `compass-core::extension_install`
use. Then writes a plan (`vicinae conformance --plan`) naming each one's
pinned command.

The stores only serve the current build, so the corpus pins *which*
extensions and commands rather than their bytes. A build whose commit or
checksum differs from the pin is reported, so a changed result can be told
apart from a changed extension.

A download that fails is reported and left out of the plan rather than
stopping the rest; the harness then counts it as not installed.
"""

import argparse
import io
import json
import pathlib
import shutil
import sys
import time
import urllib.error
import urllib.request
import zipfile

HERE = pathlib.Path(__file__).resolve().parent
RAYCAST = "https://backend.raycast.com/api/v1"
VICINAE = "https://api.vicinae.com/v1"
HEADERS = {"User-Agent": "compass-suite1"}


def get(url, as_json=True, attempts=6):
    """GETs `url`, backing off on the 429s and 503s a store gives a burst."""
    for attempt in range(attempts):
        request = urllib.request.Request(url, headers=HEADERS)
        try:
            with urllib.request.urlopen(request, timeout=120) as response:
                data = response.read()
            return json.loads(data) if as_json else data
        except urllib.error.HTTPError as err:
            if err.code not in (429, 500, 502, 503, 504) or attempt == attempts - 1:
                raise
        except urllib.error.URLError:
            if attempt == attempts - 1:
                raise
        time.sleep(2 ** attempt)
    raise AssertionError("unreachable")


def unpack(archive, target):
    """Unpacks, dropping the one top-level directory every bundle has."""
    if target.exists():
        shutil.rmtree(target)
    target.mkdir(parents=True)
    with zipfile.ZipFile(io.BytesIO(archive)) as bundle:
        for member in bundle.infolist():
            parts = pathlib.PurePosixPath(member.filename).parts[1:]
            if not parts or any(part == ".." for part in parts):
                continue
            out = target.joinpath(*parts)
            if member.is_dir():
                out.mkdir(parents=True, exist_ok=True)
                continue
            out.parent.mkdir(parents=True, exist_ok=True)
            out.write_bytes(bundle.read(member))
    if not (target / "package.json").is_file():
        raise RuntimeError("the bundle has no package.json")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("extensions_dir", type=pathlib.Path)
    parser.add_argument("plan", type=pathlib.Path)
    parser.add_argument("--only", choices=["raycast", "vicinae"])
    parser.add_argument("--refresh", action="store_true",
                        help="download again what is already installed")
    parser.add_argument("--corpus", type=pathlib.Path, default=HERE / "corpus.json")
    args = parser.parse_args()

    corpus = json.loads(args.corpus.read_text())
    args.extensions_dir.mkdir(parents=True, exist_ok=True)
    vicinae_listing = None
    plan, failed, drift = [], [], []

    entries = []
    if args.only in (None, "raycast"):
        entries += corpus["raycast"]
    if args.only in (None, "vicinae"):
        entries += corpus["vicinae"]

    for entry in entries:
        store, name = entry["store"], entry["name"]
        directory = f"store.{store}.{name}"
        target = args.extensions_dir / directory
        try:
            if (target / "package.json").is_file() and not args.refresh:
                plan.append({
                    "extension": directory,
                    "command": entry["command"],
                    "note": f"{store} store, {entry.get('downloads', '?')} installs",
                })
                continue
            if store == "raycast":
                listing = get(f"{RAYCAST}/extensions/{entry['owner']}/{name}")
                url = listing["download_url"]
                if entry.get("commit") and listing.get("commit_sha") != entry["commit"]:
                    drift.append(f"{directory}: pinned {entry['commit'][:10]}, "
                                 f"store has {str(listing.get('commit_sha'))[:10]}")
            else:
                if vicinae_listing is None:
                    vicinae_listing = {
                        e["name"]: e
                        for e in get(f"{VICINAE}/store/list?page=1&limit=500")["extensions"]
                    }
                listing = vicinae_listing[name]
                url = listing["downloadUrl"]
                if entry.get("checksum") and listing.get("checksum") != entry["checksum"]:
                    drift.append(f"{directory}: checksum changed since the pin")
            unpack(get(url, as_json=False), target)
        except Exception as err:  # noqa: BLE001 -- reported, and the rest go on
            failed.append(f"{directory}: {err}")
            continue
        plan.append({
            "extension": directory,
            "command": entry["command"],
            "note": f"{store} store, {entry.get('downloads', '?')} installs",
        })
        print(f"installed {directory}", file=sys.stderr)

    inputs = json.loads((HERE / "inputs.json").read_text())
    for entry in plan:
        for field, value in inputs.get(entry["extension"], {}).items():
            entry[field] = value
    args.plan.parent.mkdir(parents=True, exist_ok=True)
    args.plan.write_text(json.dumps({"commands": plan}, indent=2) + "\n")
    for line in drift:
        print(f"drift: {line}", file=sys.stderr)
    for line in failed:
        print(f"::warning::could not install {line}", file=sys.stderr)
    print(f"{len(plan)} installed, {len(failed)} failed, {len(drift)} drifted", file=sys.stderr)


if __name__ == "__main__":
    main()
