#!/usr/bin/env python3
"""Host entrypoint: one isolated container workflow for local runs and CI."""
import argparse
import hashlib
import json
import pathlib
import platform
import shutil
import subprocess
import tempfile

ROOT = pathlib.Path(__file__).resolve().parents[2]
IMAGE = "localhost/compass-head-to-head:v1"
CACHE = "compass-head-to-head-cargo-v1"


def command(args, **kwargs):
    print("+", " ".join(map(str, args)), flush=True)
    return subprocess.run(args, check=True, **kwargs)


def source_identity():
    def git(*args):
        return subprocess.check_output(["git", "-C", str(ROOT), *args])
    names = git("ls-files", "-z", "--cached", "--others", "--exclude-standard").split(b"\0")
    digest = hashlib.sha256()
    for name in sorted(set(filter(None, names))):
        path = ROOT / name.decode()
        if path.is_file():
            digest.update(name + b"\0" + path.read_bytes() + b"\0")
    return {"commit": git("rev-parse", "HEAD").decode().strip(),
            "dirty": bool(git("status", "--porcelain")), "source_sha256": digest.hexdigest()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=pathlib.Path, default=ROOT / "target/head-to-head")
    args = parser.parse_args()
    if platform.system() != "Linux" or platform.machine() not in ("x86_64", "AMD64"):
        parser.error("the pinned release recipe currently requires Linux x86_64")
    if not shutil.which("podman"):
        parser.error("install Podman first; no host Rust, Qt, X server or running launcher is needed")
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    run = pathlib.Path(tempfile.mkdtemp(prefix="run-", dir=output))
    print(f"Reports and failure logs: {run}", flush=True)
    provenance = source_identity()
    provenance["baseline"] = json.loads((ROOT / "scripts/bench/baseline.json").read_text())
    (run / "provenance.json").write_text(json.dumps(provenance, indent=2) + "\n")
    # A tiny build context avoids sending the repository's target/ or user files.
    with tempfile.TemporaryDirectory(prefix="compass-bench-image-") as context:
        shutil.copy2(ROOT / "scripts/bench/Containerfile", pathlib.Path(context) / "Containerfile")
        command(["podman", "build", "--tag", IMAGE, context])
    image = subprocess.check_output(["podman", "image", "inspect", IMAGE, "--format", "{{.Id}}"], text=True).strip()
    provenance["container_image_id"] = image
    (run / "provenance.json").write_text(json.dumps(provenance, indent=2) + "\n")
    common = ["podman", "run", "--rm", "--init", "--cpus=4", "--memory=8g", "--shm-size=1g",
              "-v", f"{ROOT}:/src:ro,z", "-v", f"{run}:/results:Z", "-v", f"{CACHE}:/cache"]
    # Builds/downloads may use the network. Measured processes cannot.
    command([*common, image, "python3", "scripts/bench/session.py", "prepare"])
    if source_identity()["source_sha256"] != provenance["source_sha256"]:
        raise RuntimeError("source changed during preparation; rerun before interpreting measurements")
    command([*common, "--network=none", image, "python3", "scripts/bench/session.py", "measure"])
    if source_identity()["source_sha256"] != provenance["source_sha256"]:
        raise RuntimeError("source changed during measurement; this run is not a valid baseline")
    print((run / "summary.md").read_text())
    print(f"Saved: {run}")


if __name__ == "__main__":
    main()
