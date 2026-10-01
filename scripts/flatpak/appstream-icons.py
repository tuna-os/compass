#!/usr/bin/env python3
"""Make Compass's icon reachable from a software centre on the TunaOS OCI remote.

An OCI remote has no appstream branch. Flatpak builds the remote's catalogue
from the image labels instead (flatpak_oci_index_make_appstream): the
`org.freedesktop.appstream.appdata` XML, plus the `icon-64` and `icon-128` PNGs,
which it writes to `appstream/<remote>/<arch>/icons/<size>/<id>.png`.

An ostree remote's appstream branch puts the same PNGs under
`icons/<size>/` AND links `icons/flatpak/<size>` to them
(flatpak-repo-utils.c), and that second path is the only one Bazaar looks in
for a `cached` icon: `<appstream dir>/icons/flatpak/<size>/<file>`
(bz-appstream-parser.c). On an OCI remote it does not exist, so every
`cached` icon is skipped. The only other source Bazaar takes is a `remote`
icon, an http(s) URL it downloads itself. `appstreamcli compose` drops any
icon written in the metainfo and adds `remote` ones only when it is given a
media URL, which flatpak-builder does not do. So the publish workflow adds
them to the appdata label after the build, pointing at the PNGs committed
under packaging/flatpak/icons/, and pinned to the commit that was built.

Those PNGs are the ones `appstreamcli compose` renders from the SVG.
`check-build` holds them to the build's own output, so changing the SVG
without updating them fails CI with the command that fixes it.

Usage:
  appstream-icons.py check-build <app files dir>
  appstream-icons.py add-remote  <oci dir> --url-base URL
  appstream-icons.py check-oci   <oci dir> [--url-base URL] [--fetch]

Standard library only: it runs on a bare CI runner.
"""

import argparse
import base64
import gzip
import hashlib
import json
import pathlib
import struct
import sys
import urllib.request
import xml.etree.ElementTree as ET

APP_ID = "org.tunaos.compass"
ROOT = pathlib.Path(__file__).resolve().parents[2]
COMMITTED = ROOT / "packaging/flatpak/icons"
SIZES = (64, 128)
APPDATA_LABEL = "org.freedesktop.appstream.appdata"
PNG_MAGIC = b"\x89PNG\r\n\x1a\n"


class Failure(Exception):
    """A check that did not hold, with the message CI shows."""


def png_size(data: bytes) -> tuple[int, int]:
    if not data.startswith(PNG_MAGIC) or data[12:16] != b"IHDR":
        raise Failure("not a PNG")
    return struct.unpack(">II", data[16:24])


def committed_png(size: int) -> bytes:
    path = COMMITTED / f"{size}x{size}" / f"{APP_ID}.png"
    if not path.is_file():
        raise Failure(f"{path.relative_to(ROOT)} is missing")
    return path.read_bytes()


def component(xml_text: str) -> ET.Element:
    found = [c for c in ET.fromstring(xml_text).iter("component") if c.findtext("id") == APP_ID]
    if len(found) != 1:
        raise Failure(f"expected one <component> for {APP_ID}, found {len(found)}")
    return found[0]


def icons(cpt: ET.Element, kind: str) -> dict[int, ET.Element]:
    """Icons of one kind at scale 1, by size."""
    return {
        int(i.get("width", 0)): i
        for i in cpt.findall("icon")
        if i.get("type") == kind and i.get("scale", "1") == "1"
    }


def check_build(files: pathlib.Path) -> None:
    app_info = files / "share/app-info"
    xml = gzip.decompress((app_info / f"xmls/{APP_ID}.xml.gz").read_bytes()).decode()
    cached = icons(component(xml), "cached")
    for size in SIZES:
        if size not in cached:
            raise Failure(f"the composed AppStream has no cached {size}x{size} icon")
        built = app_info / f"icons/flatpak/{size}x{size}" / cached[size].text.strip()
        data = built.read_bytes()
        if png_size(data) != (size, size):
            raise Failure(f"{built} is {png_size(data)}, not {size}x{size}")
        if data != committed_png(size):
            raise Failure(
                f"packaging/flatpak/icons/{size}x{size}/{APP_ID}.png is not the icon this build "
                f"composed from the SVG. Copy it from the build:\n"
                f"  cp {built} packaging/flatpak/icons/{size}x{size}/{APP_ID}.png"
            )
        print(f"cached {size}x{size}: {built.name}, matches the committed PNG")


def oci_config(oci: pathlib.Path) -> tuple[dict, dict, dict]:
    index = json.loads((oci / "index.json").read_text())
    blobs = oci / "blobs/sha256"
    manifest = json.loads((blobs / index["manifests"][0]["digest"].split(":")[1]).read_bytes())
    config = json.loads((blobs / manifest["config"]["digest"].split(":")[1]).read_bytes())
    return index, manifest, config


def write_blob(oci: pathlib.Path, data: bytes) -> dict:
    digest = hashlib.sha256(data).hexdigest()
    (oci / "blobs/sha256" / digest).write_bytes(data)
    return {"digest": f"sha256:{digest}", "size": len(data)}


def remote_url(url_base: str, size: int) -> str:
    return f"{url_base.rstrip('/')}/{size}x{size}/{APP_ID}.png"


def add_remote(oci: pathlib.Path, url_base: str) -> None:
    index, manifest, config = oci_config(oci)
    labels = config["config"]["Labels"]
    appdata = labels[APPDATA_LABEL]
    if icons(component(appdata), "remote"):
        raise Failure("the appdata already has remote icons; refusing to add a second set")

    # A text insertion after the last <icon>, so the rest of the XML is left
    # byte for byte as flatpak wrote it.
    last = appdata.rindex("</icon>") + len("</icon>")
    indent = appdata[appdata.rindex("\n", 0, last) + 1 :].split("<", 1)[0]
    added = "".join(
        f'\n{indent}<icon type="remote" width="{s}" height="{s}">{remote_url(url_base, s)}</icon>'
        for s in SIZES
    )
    labels[APPDATA_LABEL] = appdata[:last] + added + appdata[last:]
    component(labels[APPDATA_LABEL])

    old = [manifest["config"]["digest"], index["manifests"][0]["digest"]]
    manifest["config"].update(write_blob(oci, json.dumps(config).encode()))
    index["manifests"][0].update(write_blob(oci, json.dumps(manifest).encode()))
    (oci / "index.json").write_text(json.dumps(index))
    for digest in old:
        (oci / "blobs/sha256" / digest.split(":")[1]).unlink()
    print(f"added remote icons under {url_base}; manifest is now {index['manifests'][0]['digest']}")


def check_oci(oci: pathlib.Path, url_base: str | None, fetch: bool) -> None:
    _, _, config = oci_config(oci)
    labels = config["config"]["Labels"]
    cpt = component(labels[APPDATA_LABEL])
    cached, remote = icons(cpt, "cached"), icons(cpt, "remote")
    for size in SIZES:
        label = labels.get(f"org.freedesktop.appstream.icon-{size}", "")
        prefix = "data:image/png;base64,"
        if not label.startswith(prefix):
            raise Failure(f"no PNG icon-{size} label")
        data = base64.b64decode(label[len(prefix) :])
        if png_size(data) != (size, size):
            raise Failure(f"the icon-{size} label is {png_size(data)}, not {size}x{size}")
        # With --fetch the URL is the reference: the checkout running this may
        # be newer than the commit that was built.
        if not fetch and data != committed_png(size):
            raise Failure(f"the icon-{size} label is not the committed {size}x{size} PNG")
        if size not in cached:
            raise Failure(f"the appdata label has no cached {size}x{size} icon")
        if url_base is not None:
            want = remote_url(url_base, size)
            got = remote[size].text.strip() if size in remote else None
            if got != want:
                raise Failure(f"remote {size}x{size} icon is {got!r}, expected {want!r}")
            if fetch:
                with urllib.request.urlopen(want, timeout=30) as response:
                    if response.read() != data:
                        raise Failure(f"{want} does not serve the icon-{size} label's PNG")
        elif fetch:
            raise Failure("--fetch needs --url-base")
        print(f"icon-{size} label, cached{', remote' if url_base else ''} {size}x{size}: ok")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = parser.add_subparsers(dest="command", required=True)
    build = sub.add_parser("check-build")
    build.add_argument("files", type=pathlib.Path)
    add = sub.add_parser("add-remote")
    add.add_argument("oci", type=pathlib.Path)
    add.add_argument("--url-base", required=True)
    check = sub.add_parser("check-oci")
    check.add_argument("oci", type=pathlib.Path)
    check.add_argument("--url-base")
    check.add_argument("--fetch", action="store_true")
    args = parser.parse_args()
    try:
        if args.command == "check-build":
            check_build(args.files)
        elif args.command == "add-remote":
            add_remote(args.oci, args.url_base)
        else:
            check_oci(args.oci, args.url_base, args.fetch)
    except Failure as failure:
        print(f"::error::{failure}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
