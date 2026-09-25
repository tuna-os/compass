#!/usr/bin/env python3
"""Measure what ties Raycast store extensions to macOS.

    scripts/suite1/macos_signals.py <work-dir> [--pages 3]

Downloads the prebuilt bundles of the top `100 * pages` Raycast store
listings by installs (the bundles a user's install gets, as fetch.py does),
and counts, per extension, the macOS-specific things their JavaScript does:
a Homebrew prefix, `open`, `pbcopy`, `osascript` and AppleScript source,
`defaults`, `mdfind`, `~/Library` paths, `/Applications`, other macOS-only
programs, `process.platform === "darwin"`, and Mach-O binaries shipped
alongside. The counts behind docs/rust-engine/RAYCAST-LINUX-SHIM.md.

The bundles are minified, and library code comes with them, so a match is
a signal, not a proof: the patterns below drop the known library noise
(@raycast/utils' `runAppleScript`, execa's shell list, the `open` package's
`-a` handling, the home-directory helper's darwin branch) and count
@raycast/utils' AppleScript runner separately, since a bundle carries it
whether or not the extension calls it. Run by hand; it uses the network.
"""

import argparse
import collections
import io
import json
import pathlib
import re
import sys
import urllib.request
import zipfile

RAYCAST = "https://backend.raycast.com/api/v1"
VICINAE = "https://api.vicinae.com/v1"
HEADERS = {"User-Agent": "compass-suite1"}

UTILS_APPLESCRIPT = "AppleScript is only supported on macOS"
UTILS_SPAWN = re.compile(
    r'spawn\("osascript",\w+,\{\.\.\.\w+,env:\{PATH:"/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"\}'
)
NOISE = [
    UTILS_SPAWN,
    re.compile(r'command:"osascript",options:'),
    re.compile(r'"node","osascript","python"'),
    re.compile(r'\.push\("-a",'),
    re.compile(r'process\.platform==="darwin"\?\w+\|\|\(\w+\?"/Users/"'),
]

CATEGORIES = {
    "homebrew-prefix": re.compile(r"/opt/homebrew|/usr/local/(?:bin/brew|Cellar|Caskroom|Homebrew)"),
    "usr-local-bin": re.compile(r"/usr/local/bin/(?!brew)[A-Za-z]"),
    "open-command": re.compile(
        r"""[`"']open (?:-[abRWnFgjhe]|[`"'$]|\\?["'])|[`"']open[`"'],\s*\[|["']/usr/bin/open["']"""
    ),
    "open-a-app": re.compile(r"""[`"']open -a |["']-a["']"""),
    "pbcopy/pbpaste": re.compile(r"\bpb(?:copy|paste)\b"),
    "osascript-direct": re.compile(r"\bosascript\b"),
    "applescript-source": re.compile(r"tell application|Application\([\"'][A-Z]"),
    "defaults": re.compile(r"""[`"']defaults (?:read|write|export|import|delete)"""),
    "mdfind/mdls": re.compile(r"\bmd(?:find|ls)\b"),
    "library-paths": re.compile(
        r"Library/(?:Application Support|Preferences|Caches|Containers|Group Containers|Safari|Mail|"
        r"Messages|Keychains|Cookies)"
        r"""|["']Library["'],\s*["'](?:Application Support|Preferences|Caches|Containers|Group Containers)"""
    ),
    "applications-dir": re.compile(r"/Applications/|\.app/Contents"),
    "keychain-security-cli": re.compile(r"""[`"']security (?:find|add|delete)-"""),
    "other-macos-cli": re.compile(
        r"""[`"'](?:sw_vers|system_profiler|screencapture|afplay|say |shortcuts run|networksetup|pmset|"""
        r"""caffeinate|launchctl|lsappinfo|plutil|sips|textutil|scutil|diskutil|ioreg)\b"""
    ),
    "darwin-check": re.compile(
        r"""process\.platform\s*[!=]==?\s*["']darwin["']|["']darwin["']\s*[!=]==?\s*process\.platform"""
    ),
}
EXTRA = ["utils-runAppleScript-bundled", "macho-binary"]
APPLESCRIPT = {"osascript-direct", "applescript-source"}
SHIM_FIXES = {"homebrew-prefix", "open-command", "open-a-app", "pbcopy/pbpaste"}


def get(url, as_json=True):
    request = urllib.request.Request(url, headers=HEADERS)
    with urllib.request.urlopen(request, timeout=120) as response:
        data = response.read()
    return json.loads(data) if as_json else data


def scan(archive):
    hits = collections.Counter()
    with zipfile.ZipFile(io.BytesIO(archive)) as bundle:
        for member in bundle.infolist():
            if member.is_dir():
                continue
            name = member.filename
            if name.endswith((".applescript", ".scpt")):
                hits["applescript-source"] += 1
            if name.endswith((".js", ".cjs", ".mjs", ".sh")):
                text = bundle.read(member).decode("utf-8", "replace")
                if UTILS_APPLESCRIPT in text or UTILS_SPAWN.search(text):
                    hits["utils-runAppleScript-bundled"] += 1
                for noise in NOISE:
                    text = noise.sub("<noise>", text)
                for category, pattern in CATEGORIES.items():
                    found = pattern.findall(text)
                    if category == "darwin-check" and UTILS_APPLESCRIPT in text:
                        found = found[1:]
                    hits[category] += len(found)
            elif member.file_size > 1024:
                magic = bundle.open(member).read(4)
                if magic in (b"\xcf\xfa\xed\xfe", b"\xce\xfa\xed\xfe", b"\xca\xfe\xba\xbe"):
                    hits["macho-binary"] += 1
    return {k: v for k, v in hits.items() if v}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("work_dir", type=pathlib.Path)
    parser.add_argument("--pages", type=int, default=3)
    args = parser.parse_args()
    args.work_dir.mkdir(parents=True, exist_ok=True)

    listings = []
    for page in range(1, args.pages + 1):
        listings += get(f"{RAYCAST}/store_listings?per_page=100&page={page}")["data"]
    compat = get(f"{VICINAE}/raycast/get-compat")

    results = {}
    for listing in listings:
        name = listing["name"]
        try:
            hits = scan(get(listing["download_url"], as_json=False))
        except Exception as err:  # noqa: BLE001 -- reported, the rest go on
            print(f"could not scan {name}: {err}", file=sys.stderr)
            continue
        results[name] = {
            "hits": hits,
            "platforms": listing.get("platforms") or [],
            "downloads": listing["download_count"],
            "compat": compat.get(name, {}).get("status"),
        }
    (args.work_dir / "macos-signals.json").write_text(json.dumps(results, indent=1) + "\n")

    def signals(name):
        return {c for c in CATEGORIES if c in results[name]["hits"]} | (
            {"macho-binary"} & set(results[name]["hits"])
        )

    print(f"{len(results)} extensions; platforms:",
          dict(collections.Counter(tuple(r["platforms"]) for r in results.values())))
    print(f"{'category':30} {'all':>5} {'macOS-only':>11}")
    for category in [*CATEGORIES, *EXTRA]:
        having = [n for n, r in results.items() if category in r["hits"]]
        mac = [n for n in having if results[n]["platforms"] == ["macOS"]]
        print(f"{category:30} {len(having):5} {len(mac):11}")
    anything = [n for n in results if signals(n)]
    print("any macOS signal:", len(anything))
    print("AppleScript or JXA:", sum(1 for n in results if signals(n) & APPLESCRIPT))
    print("only what the shim fixes:", sorted(n for n in anything if signals(n) <= SHIM_FIXES))


if __name__ == "__main__":
    main()
