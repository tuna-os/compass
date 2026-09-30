#!/usr/bin/env python3
"""Pick Suite 1's Raycast corpus, and write it to corpus.json.

THE CRITERION (PLAN §8.2 says "top 25 Raycast store extensions by installs")

  1. Every listing on the first three pages of the Raycast store API, sorted
     by `download_count`, highest first. Three pages of 100 reach well past the
     25th Linux-eligible extension; the store sorts by installs itself, the
     re-sort only makes that explicit.
  2. Minus what cannot run on Linux at all: extensions that Vicinae's own
     compatibility sheet (api.vicinae.com/v1/raycast/get-compat, the one its
     store view shows) marks `impossible` — AppleScript, Swift, a macOS-only
     application — and extensions whose listing names macOS as their only
     platform. Both are properties of the extension, not of the host, and
     running them would measure macOS rather than Compass.
  3. The first 25 that remain.

Everything else is kept, including extensions the sheet marks `nothing` or
`partial`: those are host gaps, which is what Suite 1 exists to find.

For each, the command is the first `view` command in the manifest's order,
else the first command: one command each, as §8.2 asks.

This is run by hand when the corpus is re-pinned, not in CI: a corpus that
changes under the suite would turn a regression into a shrug. The date and
each extension's commit are recorded so a re-pin says what moved.

    scripts/suite1/select.py > scripts/suite1/corpus.json
"""

import datetime
import json
import sys
import urllib.request

RAYCAST = "https://backend.raycast.com/api/v1"
VICINAE = "https://api.vicinae.com/v1"
TOP = 25
PAGES = 3


def get(url):
    # api.vicinae.com refuses Python's default User-Agent.
    request = urllib.request.Request(url, headers={"User-Agent": "compass-suite1"})
    with urllib.request.urlopen(request, timeout=60) as response:
        return json.load(response)


def first_command(commands):
    views = [c for c in commands if c.get("mode") == "view"]
    return (views or commands)[0]["name"] if commands else None


def main():
    listings = []
    for page in range(1, PAGES + 1):
        listings += get(f"{RAYCAST}/store_listings?per_page=100&page={page}")["data"]
    listings.sort(key=lambda e: -e["download_count"])
    compat = get(f"{VICINAE}/raycast/get-compat")

    picked, skipped = [], []
    for e in listings:
        sheet = compat.get(e["name"], {})
        platforms = e.get("platforms") or []
        if sheet.get("status") == "impossible":
            skipped.append({"name": e["name"], "why": "compat sheet: impossible",
                            "notes": sheet.get("notes", [])})
            continue
        if platforms == ["macOS"]:
            skipped.append({"name": e["name"], "why": "listed for macOS only"})
            continue
        picked.append({
            "store": "raycast",
            "name": e["name"],
            "author": e["author"]["handle"],
            # The store addresses a listing by its owner, which for an
            # extension maintained by a company is the company, not the author.
            "owner": e["owner"]["handle"],
            "title": e["title"],
            "downloads": e["download_count"],
            "commit": e.get("commit_sha"),
            "command": first_command(e.get("commands") or []),
            "compat": sheet.get("status"),
        })
        if len(picked) == TOP:
            break

    vicinae = get(f"{VICINAE}/store/list?page=1&limit=500")["extensions"]
    vicinae.sort(key=lambda e: -e["downloadCount"])
    corpus = {
        "pinned": datetime.date.today().isoformat(),
        "criterion": "Raycast: top 25 by download_count over the first 300 listings, "
                     "excluding compat-sheet 'impossible' and macOS-only listings; "
                     "Vicinae: every store extension. First view command of each.",
        "raycast": picked,
        "raycast_skipped": skipped,
        "vicinae": [
            {
                "store": "vicinae",
                "name": e["name"],
                "author": e["author"]["handle"],
                "title": e["title"],
                "downloads": e["downloadCount"],
                "checksum": e.get("checksum"),
                "command": first_command(e.get("commands") or []),
            }
            for e in vicinae
        ],
    }
    json.dump(corpus, sys.stdout, indent=2)
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()
