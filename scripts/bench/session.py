#!/usr/bin/env python3
"""Container-side build and benchmark lifecycle. Never targets the user's session."""
import contextlib
import hashlib
import json
import os
import pathlib
import shutil
import signal
import socket
import statistics
import struct
import subprocess
import sys
import time
import urllib.request

ROOT = pathlib.Path("/src")
RESULTS = pathlib.Path("/results")
CACHE = pathlib.Path("/cache")


def sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def checked(args, **kwargs):
    print("+", " ".join(map(str, args)), flush=True)
    return subprocess.run(args, check=True, **kwargs)


def prepare():
    baseline = json.loads((ROOT / "scripts/bench/baseline.json").read_text())
    CACHE.mkdir(exist_ok=True)
    archive = CACHE / f"Vicinae-{baseline['tag']}.AppImage"
    if not archive.exists():
        partial = archive.with_suffix(".download")
        with urllib.request.urlopen(baseline["url"], timeout=120) as response, partial.open("wb") as out:
            shutil.copyfileobj(response, out)
        if sha256(partial) != baseline["sha256"]:
            raise RuntimeError("upstream release checksum mismatch")
        partial.rename(archive)
    if sha256(archive) != baseline["sha256"]:
        raise RuntimeError("cached upstream release checksum mismatch")
    archive.chmod(0o755)
    # Extraction into the unique run directory prevents stale extracted files.
    with (RESULTS / "extract.log").open("w") as log:
        checked([str(archive), "--appimage-extract"], cwd=RESULTS, stdout=log, stderr=subprocess.STDOUT)
    env = dict(os.environ, PATH="/root/.rustup/toolchains/1.94.1-x86_64-unknown-linux-gnu/bin:" + os.environ["PATH"])
    with (RESULTS / "build.log").open("w") as log:
        print("Building release Rust binaries; progress is in build.log", flush=True)
        checked(["cargo", "build", "--release", "--locked", "-p", "vicinae", "-p", "compass-testkit",
                 "--bin", "vicinae", "--bin", "head-to-head"], cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT)
    for binary in ("vicinae", "head-to-head"):
        shutil.copy2(CACHE / "target/release" / binary, RESULTS / binary)
    (RESULTS / "toolchain.txt").write_text(subprocess.check_output(["rustc", "--version", "--verbose"], env=env, text=True))
    (RESULTS / "packages.txt").write_text(subprocess.check_output(["rpm", "-qa"], text=True))


def rpc(path, method):
    payload = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": {}}).encode()
    with socket.socket(socket.AF_UNIX) as stream:
        stream.settimeout(1)
        stream.connect(str(path))
        stream.sendall(struct.pack("=I", len(payload)) + payload)
        def read(size):
            data = b""
            while len(data) < size:
                chunk = stream.recv(size - len(data))
                if not chunk:
                    raise RuntimeError("upstream socket closed mid-frame")
                data += chunk
            return data
        size = struct.unpack("=I", read(4))[0]
        if size > 16 * 1024 * 1024:
            raise RuntimeError("oversized upstream response")
        answer = json.loads(read(size))
        if answer.get("id") != 1 or "error" in answer or "result" not in answer:
            raise RuntimeError(f"bad upstream response: {answer}")
        return answer["result"]


def wait_ready(probe, processes, timeout=30):
    deadline = time.monotonic() + timeout
    last = None
    while time.monotonic() < deadline:
        for process in processes:
            if process.poll() is not None:
                raise RuntimeError(f"process {process.args} exited {process.returncode}; inspect logs")
        try:
            value = probe()
            if value:
                return value
        except (OSError, RuntimeError, subprocess.SubprocessError) as error:
            last = error
        time.sleep(0.1)
    raise RuntimeError(f"readiness deadline exceeded: {last}")


@contextlib.contextmanager
def processes():
    children = []
    logs = []
    def start(name, args, env):
        log = (RESULTS / f"{name}.log").open("w")
        logs.append(log)
        child = subprocess.Popen(args, env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        children.append(child)
        return child
    try:
        yield children, start
    finally:
        # Only groups created above are signalled. Reap even on failed readiness.
        for child in reversed(children):
            with contextlib.suppress(ProcessLookupError):
                os.killpg(child.pid, signal.SIGTERM)
        for child in children:
            try:
                child.wait(timeout=5)
            except subprocess.TimeoutExpired:
                with contextlib.suppress(ProcessLookupError):
                    os.killpg(child.pid, signal.SIGKILL)
                child.wait()
        for log in logs:
            log.close()


def profile(name):
    env = dict(os.environ, DISPLAY=":99", QT_QPA_PLATFORM="xcb", LIBGL_ALWAYS_SOFTWARE="1")
    for key, directory in (("XDG_CONFIG_HOME", "config"), ("XDG_DATA_HOME", "data"),
                           ("XDG_STATE_HOME", "state"), ("XDG_CACHE_HOME", "cache"), ("XDG_RUNTIME_DIR", "runtime")):
        path = RESULTS / name / directory
        path.mkdir(parents=True, exist_ok=True, mode=0o700)
        env[key] = str(path)
    env["XDG_DATA_DIRS"] = str(RESULTS / "corpus")
    env["LC_ALL"] = "C.UTF-8"
    return env


def visible(pid, env, title):
    result = subprocess.run(["xdotool", "search", "--onlyvisible", "--pid", str(pid)], env=env,
                            capture_output=True, text=True, timeout=2, check=False)
    if result.returncode != 0:
        return []
    windows = []
    for window_id in result.stdout.split():
        name = subprocess.run(["xdotool", "getwindowname", window_id], env=env,
                              capture_output=True, text=True, timeout=2, check=False)
        if name.returncode == 0 and name.stdout.strip() == title:
            windows.append({"id": window_id, "title": title, "pid": pid})
    return windows


def summarize(reports):
    lines = ["# Upstream release comparison", "", "Exploratory X11/software-rendering workload, not a GNOME/Flatpak cutover gate.", "",
             "| Run | Upstream ping median (µs) | Rust ping median (µs) | Upstream PSS (KiB) | Rust PSS (KiB) |", "|---|---:|---:|---:|---:|"]
    for index, report in enumerate(reports, 1):
        latency = report["latency"][0]
        memory = report["memory"]
        lines.append(f"| {index} | {latency['cpp']['median']/1000:.3f} | {latency['rust']['median']/1000:.3f} | "
                     f"{statistics.median(x['pss_kib'] for x in memory['cpp']):.0f} | {statistics.median(x['pss_kib'] for x in memory['rust']):.0f} |")
    lines += ["", "Search is excluded: the unmodified upstream release has no rootQuery endpoint.",
              "Memory is diagnostic, not a feature-equivalent efficiency claim: upstream has services the port lacks.",
              "Both launcher windows were verified mapped; Rust daemon and UI are counted. No TypeScript extensions.",
              "No first-frame, launch or peak-memory claim. Raw samples, logs, packages and binary hashes accompany this summary."]
    return "\n".join(lines) + "\n"


def measure():
    provenance = json.loads((RESULTS / "provenance.json").read_text())
    baseline = provenance["baseline"]
    corpus = RESULTS / "corpus/applications"
    shutil.copytree(ROOT / "crates/compass-testkit/corpus/desktop-entries/real", corpus)
    corpus_hash = hashlib.sha256()
    entries = sorted(corpus.glob("*.desktop"))
    for entry in entries:
        corpus_hash.update(entry.name.encode() + b"\0" + entry.read_bytes() + b"\0")
    if len(entries) < 100:
        raise RuntimeError("corpus missing or unexpectedly small")
    cpp_env, rust_env = profile("cpp"), profile("rust")
    state = pathlib.Path(cpp_env["XDG_STATE_HOME"]) / "vicinae"
    state.mkdir(exist_ok=True)
    (state / "onboarding.json").write_text(json.dumps({"version": 1, "completedAt": "2026-09-20T00:00:00Z"}))
    config = RESULTS / "upstream-config.json"
    config.write_text(json.dumps({"telemetry": {"system_info": False}}))
    cpp = RESULTS / "squashfs-root/AppRun"
    rust = RESULTS / "vicinae"
    cpp_socket = pathlib.Path(cpp_env["XDG_RUNTIME_DIR"]) / "vicinae/vicinae.sock"
    rust_socket = pathlib.Path(rust_env["XDG_RUNTIME_DIR"]) / "vicinae/ipc.sock"
    with processes() as (children, start):
        start("xvfb", ["Xvfb", ":99", "-screen", "0", "1280x800x24", "-nolisten", "tcp"], os.environ)
        wait_ready(lambda: subprocess.run(["xdotool", "getdisplaygeometry"], env=cpp_env,
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=2, check=False).returncode == 0, children)
        start("upstream", [str(cpp), "server", "--config", str(config), "--no-extension-runtime", "--open"], cpp_env)
        cpp_pid = wait_ready(lambda: rpc(cpp_socket, "Ipc/ping").get("pid"), children)
        rust_daemon = start("rust-daemon", [str(rust), "serve", "--no-hotkey"], rust_env)
        wait_ready(lambda: subprocess.run([str(rust), "ping"], env=rust_env, stdout=subprocess.DEVNULL,
                   stderr=subprocess.DEVNULL, timeout=2, check=False).returncode == 0, children)
        rust_ui = start("rust-ui", [str(rust), "ui"], rust_env)
        windows = {"cpp": wait_ready(lambda: visible(cpp_pid, cpp_env, "Vicinae Launcher"), children),
                   "rust": wait_ready(lambda: visible(rust_ui.pid, rust_env, "Vicinae"), children)}
        (RESULTS / "windows.json").write_text(json.dumps(windows, indent=2))
        workload = f"Xvfb X11 1280x800 software rendering; both windows mapped; onboarding completed; no TS extensions; network disabled; corpus={len(entries)} sha256={corpus_hash.hexdigest()}; unequal builtin service coverage; not a full memory-efficiency comparison"
        bench_config = {"cpp": {"socket": str(cpp_socket), "revision": f"{baseline['repository']}@{baseline['commit']} {baseline['tag']} unmodified",
                               "build": f"official AppImage sha256={baseline['sha256']}", "process_roots": [cpp_pid]},
                        "rust": {"socket": str(rust_socket), "revision": json.dumps(provenance, sort_keys=True),
                                 "build": f"cargo --release rustc {baseline['rust_toolchain']} sha256={sha256(rust)}", "process_roots": [rust_daemon.pid, rust_ui.pid]},
                        "workload": workload, "samples": 1000, "warmup": 100, "queries": []}
        config_path = RESULTS / "benchmark-config.json"
        config_path.write_text(json.dumps(bench_config, indent=2))
        reports = []
        for index in range(1, 4):
            report = RESULTS / f"report-{index}.json"
            checked([str(RESULTS / "head-to-head"), str(config_path), str(report)], timeout=120)
            reports.append(json.loads(report.read_text()))
        (RESULTS / "summary.md").write_text(summarize(reports))


if __name__ == "__main__":
    if sys.argv[1:] == ["prepare"]:
        prepare()
    elif sys.argv[1:] == ["measure"]:
        measure()
    else:
        raise SystemExit("usage: session.py prepare|measure (inside benchmark container)")
