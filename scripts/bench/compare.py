#!/usr/bin/env python3
"""Compass against upstream Vicinae on one machine, on a headless Wayland compositor.

Run through scripts/bench/compare.sh, which builds the inputs. Direct use:

  compare.py --compass target/release/compass --upstream squashfs-root/AppRun \
             --cpp-rank cpp-rank --fuzzy-throughput target/release/fuzzy-throughput \
             --out DIR [--runs 5]

What it measures, per engine, over --runs alternating cold starts on one
headless Sway (pixman renderer, no GPU, 1280x800):

  ready_ms       exec -> first successful IPC ping
  first_frame_ms exec -> first screenshot that differs from the empty output
  complete_ms    exec -> the last change to the picture before it idles (clock and
                 caret rows excluded), i.e. the populated launcher
  idle           process-tree RSS/PSS, threads, processes, mapped .so files,
                 IDLE_SETTLE_S after the first frame
  type_ms        `wtype` of a query -> first changed screenshot
  after_search   the same process-tree figures once the typed frame settles

plus the fuzzy scorer head-to-head (same haystack, same queries) and on-disk
sizes. Everything runs under throwaway HOME/XDG directories, a private D-Bus
session bus and a private compositor; nothing touches the caller's session.
Process trees are found by an environment tag every descendant inherits, so
detached helpers are counted too; the compositor and D-Bus daemon are not.
"""

import argparse
import contextlib
import hashlib
import json
import os
import pathlib
import platform
import shutil
import signal
import socket
import statistics
import struct
import subprocess
import sys
import tempfile
import time

ROOT = pathlib.Path(__file__).resolve().parents[2]
CORPUS = ROOT / "crates/compass-testkit/corpus/desktop-entries/real"
WIDTH, HEIGHT = 1280, 800
BACKGROUND = bytes((0x20, 0x20, 0x20))
IDLE_SETTLE_S = 5.0
DEADLINE_S = 60.0
TYPED_QUERY = "fire"
FUZZY_QUERIES = ["ed", "fire", "sysmon", "calc pro", "q", "zzzz"]
FUZZY_ITEMS = 10_000


def log(*parts):
    print(*parts, file=sys.stderr, flush=True)


def haystack(path):
    """The 10,000 names `compass-testkit/benches/slas.rs` ranks, in the same order."""
    heads = ["Firefox", "Files", "Text Editor", "System Monitor", "Calculator", "Terminal",
             "Settings", "Image Viewer", "Video Player", "Music", "Disk Usage", "Screenshot",
             "Archive Manager", "Document Viewer", "Password Manager", "Mail", "Calendar",
             "Contacts", "Weather", "Maps"]
    tails = ["", " Nightly", " Devel", " (Wayland)", " Preferences", " Beta", " Classic",
             " Extended", " Lite", " Pro"]
    lines = [f"{heads[i % len(heads)]}{tails[(i // len(heads)) % len(tails)]} {i}" for i in range(FUZZY_ITEMS)]
    path.write_text("\n".join(lines) + "\n")


def fuzzy(args, out):
    hay = out / "haystack.txt"
    haystack(hay)
    rows = {}
    cpp = subprocess.run([str(args.cpp_rank), str(hay), str(args.fuzzy_iterations), *FUZZY_QUERIES],
                         check=True, capture_output=True, text=True).stdout
    for line in cpp.splitlines():
        query, matched, median, p95 = line.split("\t")
        rows.setdefault(query, {})["upstream"] = {"matches": int(matched), "median_ns": int(median), "p95_ns": int(p95)}
    for threads, label in ((None, "compass"), ("1", "compass_1t")):
        env = dict(os.environ)
        if threads:
            env["RAYON_NUM_THREADS"] = threads
        rust = subprocess.run([str(args.fuzzy_throughput), str(hay), str(args.fuzzy_iterations), *FUZZY_QUERIES],
                              check=True, capture_output=True, text=True, env=env).stdout
        for line in rust.splitlines():
            ranker, query, matched, median, p95 = line.split("\t")
            key = f"{label}_{ranker}"
            if threads and ranker == "mirror":
                continue  # the mirror is single-threaded already
            rows[query][key] = {"matches": int(matched), "median_ns": int(median), "p95_ns": int(p95)}
    return rows


def tree_size(path):
    total = 0
    for entry in pathlib.Path(path).rglob("*"):
        if entry.is_file() and not entry.is_symlink():
            total += entry.stat().st_size
    return total


def stripped_size(binary, scratch):
    copy = scratch / (binary.name + ".stripped")
    shutil.copy2(binary, copy)
    subprocess.run(["strip", str(copy)], check=True)
    size = copy.stat().st_size
    copy.unlink()
    return size


def sizes(args, out):
    compass_bin = pathlib.Path(args.compass)
    helpers = [compass_bin.parent / name for name in ("compass-sandbox-exec", "compass-file-indexer")]
    upstream_root = pathlib.Path(args.upstream).parent
    result = {
        "compass": {
            "binary_bytes": compass_bin.stat().st_size,
            "binary_stripped_bytes": stripped_size(compass_bin, out),
            "helpers_stripped_bytes": {h.name: stripped_size(h, out) for h in helpers if h.exists()},
            "dt_needed": needed(compass_bin),
        },
        "upstream": {
            "appimage_bytes": pathlib.Path(args.upstream_appimage).stat().st_size if args.upstream_appimage else None,
            "extracted_tree_bytes": tree_size(upstream_root),
            "bundled_shared_objects": sum(1 for p in (upstream_root / "usr/lib").rglob("*.so*") if p.is_file()),
            "server_dt_needed": needed(upstream_root / "usr/bin/vicinae-server"),
        },
    }
    return result


def needed(binary):
    dump = subprocess.run(["readelf", "-d", str(binary)], capture_output=True, text=True, check=True).stdout
    return sorted(line.split("[", 1)[1].rstrip("]") for line in dump.splitlines() if "(NEEDED)" in line)


# --- processes -------------------------------------------------------------

def tagged(tag):
    needle = f"COMPASS_BENCH_TAG={tag}".encode()
    pids = []
    for entry in pathlib.Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        try:
            if needle in (entry / "environ").read_bytes().split(b"\0"):
                pids.append(int(entry.name))
        except OSError:
            continue
    return sorted(pids)


def snapshot(tag):
    pids = tagged(tag)
    rss = pss = threads = 0
    libraries = set()
    names = []
    for pid in pids:
        proc = pathlib.Path(f"/proc/{pid}")
        try:
            for line in (proc / "smaps_rollup").read_text().splitlines():
                if line.startswith("Rss:"):
                    rss += int(line.split()[1])
                elif line.startswith("Pss:"):
                    pss += int(line.split()[1])
            threads += len(list((proc / "task").iterdir()))
            names.append((proc / "comm").read_text().strip())
            for line in (proc / "maps").read_text().splitlines():
                path = line.split(maxsplit=5)[5] if len(line.split(maxsplit=5)) == 6 else ""
                if ".so" in pathlib.Path(path).name:
                    libraries.add(pathlib.Path(path).name)
        except OSError:
            continue
    return {"processes": len(pids), "names": sorted(names), "threads": threads,
            "rss_kib": rss, "pss_kib": pss, "shared_objects": len(libraries),
            "shared_object_names": sorted(libraries)}


def kill_tag(tag):
    for _ in range(50):
        pids = tagged(tag)
        if not pids:
            return
        for pid in pids:
            with contextlib.suppress(ProcessLookupError):
                os.kill(pid, signal.SIGTERM if _ < 20 else signal.SIGKILL)
        time.sleep(0.1)
    raise RuntimeError(f"processes tagged {tag} survived SIGKILL")


# --- compositor ------------------------------------------------------------

class Screen:
    def __init__(self, env):
        self.env = env
        self.empty = BACKGROUND * (WIDTH * HEIGHT)

    def grab(self):
        data = subprocess.run(["grim", "-t", "ppm", "-"], env=self.env, capture_output=True, check=True).stdout
        # grim writes a fixed "P6\n<w> <h>\n255\n" header.
        return data[data.index(b"255\n") + 4:]

    def blank(self):
        return self.grab() == self.empty


def wait(predicate, what, deadline=DEADLINE_S, children=()):
    end = time.monotonic() + deadline
    while time.monotonic() < end:
        for child in children:
            if child.poll() is not None:
                raise RuntimeError(f"{what}: {child.args[0]} exited {child.returncode}")
        value = predicate()
        if value:
            return value
        time.sleep(0.005)
    raise RuntimeError(f"timed out waiting for {what}")


ROW = WIDTH * 3


def rows_changed(a, b, ignore=frozenset()):
    return {y for y in range(HEIGHT) if y not in ignore and a[y * ROW:(y + 1) * ROW] != b[y * ROW:(y + 1) * ROW]}


def ticking_rows(screen, watch_s=2.5):
    """Rows that change with nobody typing: a footer clock, a blinking caret."""
    rows = set()
    frame = screen.grab()
    end = time.monotonic() + watch_s
    while time.monotonic() < end:
        time.sleep(0.05)
        now = screen.grab()
        rows |= rows_changed(frame, now)
        frame = now
    return frozenset(rows)


def settled(screen, ignore, quiet_s=1.0):
    """Wait until no row outside `ignore` has changed for quiet_s; return the frame."""
    frame = screen.grab()
    since = time.monotonic()
    end = since + DEADLINE_S
    while time.monotonic() < end:
        time.sleep(0.05)
        now = screen.grab()
        if rows_changed(frame, now, ignore):
            frame, since = now, time.monotonic()
        elif time.monotonic() - since >= quiet_s:
            return frame
    raise RuntimeError("the frame never settled")


# --- engines ---------------------------------------------------------------

def upstream_ping(path):
    payload = json.dumps({"jsonrpc": "2.0", "id": 1, "method": "Ipc/ping", "params": {}}).encode()
    try:
        with socket.socket(socket.AF_UNIX) as stream:
            stream.settimeout(1)
            stream.connect(str(path))
            stream.sendall(struct.pack("=I", len(payload)) + payload)
            header = stream.recv(4)
            return len(header) == 4
    except OSError:
        return False


def profile(base, name, wayland, bus):
    env = {k: v for k, v in os.environ.items()
           if k in ("PATH", "LANG", "TERM") or k.startswith("LC_")}
    for key, sub in (("HOME", "home"), ("XDG_CONFIG_HOME", "config"), ("XDG_DATA_HOME", "data"),
                     ("XDG_STATE_HOME", "state"), ("XDG_CACHE_HOME", "cache"), ("XDG_RUNTIME_DIR", "runtime")):
        path = base / name / sub
        path.mkdir(parents=True, exist_ok=True, mode=0o700)
        env[key] = str(path)
    env.update(WAYLAND_DISPLAY=wayland, DBUS_SESSION_BUS_ADDRESS=bus, XDG_DATA_DIRS=str(base / "corpus"),
               XDG_SESSION_TYPE="wayland", XDG_CURRENT_DESKTOP="sway", LC_ALL="C.UTF-8")
    return env


def launch(kind, args, base, run, wayland, bus):
    tag = f"{kind}-{run}-{os.getpid()}"
    env = profile(base, f"{kind}-{run}", wayland, bus)
    env["COMPASS_BENCH_TAG"] = tag
    logfile = (base / f"{kind}-{run}.log").open("w")
    if kind == "upstream":
        state = pathlib.Path(env["XDG_STATE_HOME"]) / "vicinae"
        state.mkdir(exist_ok=True)
        (state / "onboarding.json").write_text(json.dumps({"version": 1, "completedAt": "2026-09-20T00:00:00Z"}))
        config = base / "upstream-config.json"
        config.write_text(json.dumps({"telemetry": {"system_info": False}}))
        env["QT_QPA_PLATFORM"] = "wayland"
        sock = pathlib.Path(env["XDG_RUNTIME_DIR"]) / "vicinae/vicinae.sock"
        argv = [str(args.upstream), "server", "--config", str(config), "--no-extension-runtime", "--open"]
        ready = lambda: upstream_ping(sock)  # noqa: E731
    else:
        env["COMPASS_NO_ONBOARDING"] = "1"
        sock = pathlib.Path(env["XDG_RUNTIME_DIR"]) / "compass.sock"
        argv = [str(args.compass), "--socket", str(sock), "start"]
        ping = [str(args.compass), "--socket", str(sock), "ping"]
        ready = lambda: subprocess.run(ping, env=env, stdout=subprocess.DEVNULL,  # noqa: E731
                                       stderr=subprocess.DEVNULL, check=False).returncode == 0
    if shutil.which("unshare") and subprocess.run(["unshare", "--net", "true"], check=False).returncode == 0:
        argv = ["unshare", "--net", *argv]  # both engines offline, as upstream fetches rates at start
    start = time.monotonic()
    child = subprocess.Popen(argv, env=env, stdout=logfile, stderr=subprocess.STDOUT, start_new_session=True)
    return tag, child, start, ready, logfile


def measure_one(kind, args, base, run, screen, wayland, bus):
    if not screen.blank():
        raise RuntimeError("output not empty before launch")
    tag, child, start, ready, logfile = launch(kind, args, base, run, wayland, bus)
    result = {"engine": kind, "run": run}
    try:
        seen_ready = seen_frame = None
        end = start + DEADLINE_S
        while (seen_ready is None or seen_frame is None) and time.monotonic() < end:
            if child.poll() is not None and not tagged(tag):
                raise RuntimeError(f"{kind} exited {child.returncode}; see {logfile.name}")
            if seen_frame is None and not screen.blank():
                seen_frame = time.monotonic()
            if seen_ready is None and ready():
                seen_ready = time.monotonic()
        if seen_ready is None or seen_frame is None:
            raise RuntimeError(f"{kind}: ready={seen_ready} frame={seen_frame}; see {logfile.name}")
        result["ready_ms"] = round((seen_ready - start) * 1000, 1)
        result["first_frame_ms"] = round((seen_frame - start) * 1000, 1)
        # Keep watching while it idles: the first frame may be an empty surface,
        # so also record when the picture last changed (ticking rows excluded below).
        changes = []
        frame = screen.grab()
        while time.monotonic() - seen_frame < IDLE_SETTLE_S:
            now = screen.grab()
            changes.append((time.monotonic(), rows_changed(frame, now)))
            frame = now
        result["idle"] = snapshot(tag)
        ignore = ticking_rows(screen)
        result["ticking_rows"] = len(ignore)
        last = max((t for t, rows in changes if rows - ignore), default=seen_frame)
        result["complete_ms"] = round((last - start) * 1000, 1)
        before = settled(screen, ignore)
        subprocess.run(["grim", str(base / f"{kind}-{run}-idle.png")], env=screen.env, check=True)
        typed = time.monotonic()
        subprocess.run(["wtype", TYPED_QUERY], env=screen.env, check=True)
        wait(lambda: rows_changed(before, screen.grab(), ignore), f"{kind}: typed text on screen")
        result["type_ms"] = round((time.monotonic() - typed) * 1000, 1)
        settled(screen, ignore)
        subprocess.run(["grim", str(base / f"{kind}-{run}-typed.png")], env=screen.env, check=True)
        time.sleep(1.0)
        result["after_search"] = snapshot(tag)
    finally:
        kill_tag(tag)
        with contextlib.suppress(subprocess.TimeoutExpired):
            child.wait(timeout=5)
        logfile.close()
        wait(screen.blank, "output empty after shutdown", deadline=15)
    log(f"  {kind} run {run}: ready {result.get('ready_ms')} ms, frame {result.get('first_frame_ms')} ms, "
        f"idle PSS {result.get('idle', {}).get('pss_kib')} KiB")
    return result


@contextlib.contextmanager
def session(base):
    run_dir = base / "session-runtime"
    run_dir.mkdir(mode=0o700)
    env = {"PATH": os.environ["PATH"], "XDG_RUNTIME_DIR": str(run_dir), "HOME": str(base / "session-home"),
           "WLR_BACKENDS": "headless", "WLR_LIBINPUT_NO_DEVICES": "1", "WLR_RENDERER": "pixman"}
    (base / "session-home").mkdir()
    cfg = base / "sway.cfg"
    cfg.write_text(f"output HEADLESS-1 resolution {WIDTH}x{HEIGHT} bg #202020 solid_color\n"
                   "xwayland disable\ndefault_border none\n")
    children = []
    try:
        # No <servicedir>: nothing is bus-activated (no keyring, no portals) for either engine.
        bus_cfg = base / "bus.conf"
        bus_cfg.write_text(f"""<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig><type>session</type><listen>unix:path={run_dir}/bus</listen>
<policy context="default"><allow send_destination="*" eavesdrop="true"/><allow eavesdrop="true"/><allow own="*"/></policy>
</busconfig>
""")
        bus = subprocess.Popen(["dbus-daemon", f"--config-file={bus_cfg}", "--nofork", "--print-address=1"],
                               env=env, stdout=subprocess.PIPE, text=True)
        children.append(bus)
        address = bus.stdout.readline().strip()
        sway = subprocess.Popen(["sway", "-c", str(cfg)], env=env, stdout=(base / "sway.log").open("w"),
                                stderr=subprocess.STDOUT)
        children.append(sway)
        display = wait(lambda: next((p.name for p in run_dir.glob("wayland-*") if not p.name.endswith(".lock")), None),
                       "sway to listen", children=[sway])
        wayland = str(run_dir / display)
        # Headless wlroots has no keyboard, and Qt aborts on a seat without one.
        # A virtual keyboard held open for the session gives the seat that capability.
        keyboard = subprocess.Popen(["wtype", "-s", "86400000"], env=dict(env, WAYLAND_DISPLAY=wayland))
        children.append(keyboard)
        time.sleep(0.3)
        yield Screen(dict(env, WAYLAND_DISPLAY=wayland)), wayland, address
    finally:
        for child in reversed(children):
            child.terminate()
            with contextlib.suppress(subprocess.TimeoutExpired):
                child.wait(timeout=5)


def median_of(runs, *path):
    values = []
    for run in runs:
        value = run
        for key in path:
            value = value[key]
        values.append(value)
    return statistics.median(values)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--compass", type=pathlib.Path, required=True)
    parser.add_argument("--upstream", type=pathlib.Path, required=True, help="extracted AppImage's AppRun")
    parser.add_argument("--upstream-appimage", type=pathlib.Path)
    parser.add_argument("--cpp-rank", type=pathlib.Path, required=True)
    parser.add_argument("--fuzzy-throughput", type=pathlib.Path, required=True)
    parser.add_argument("--out", type=pathlib.Path, required=True)
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument("--fuzzy-iterations", type=int, default=500)
    args = parser.parse_args()
    for tool in ("sway", "grim", "wtype", "dbus-daemon", "readelf", "strip"):
        if not shutil.which(tool):
            parser.error(f"missing {tool}")
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)

    report = {
        "machine": {"kernel": platform.release(), "cpu": cpu_model(), "cpus": os.cpu_count(),
                    "mem_kib": mem_total(), "os": os_release(),
                    "loadavg_start": pathlib.Path("/proc/loadavg").read_text().split()[:3]},
        "compass": {"binary": str(args.compass), "sha256": sha256(args.compass),
                    "version": subprocess.run([str(args.compass), "--version"], capture_output=True,
                                              text=True).stdout.strip()},
        "upstream": {"apprun": str(args.upstream),
                     "appimage_sha256": sha256(args.upstream_appimage) if args.upstream_appimage else None},
        "method": {"runs": args.runs, "idle_settle_s": IDLE_SETTLE_S, "typed_query": TYPED_QUERY,
                   "compositor": f"sway headless, pixman, {WIDTH}x{HEIGHT}", "network": "none (unshare --net)", "corpus_entries": len(list(CORPUS.glob('*.desktop'))),
                   "fuzzy_items": FUZZY_ITEMS, "fuzzy_iterations": args.fuzzy_iterations},
    }
    log("sizes...")
    report["sizes"] = sizes(args, out)
    log("fuzzy scorer...")
    report["fuzzy"] = fuzzy(args, out)

    log("cold starts...")
    # Short on purpose: sway's IPC socket path must fit in sun_path.
    base = pathlib.Path(tempfile.mkdtemp(prefix="cc-", dir="/tmp"))
    try:
        (base / "corpus").mkdir()
        shutil.copytree(CORPUS, base / "corpus/applications")
        runs = {"compass": [], "upstream": []}
        with session(base) as (screen, wayland, bus):
            grabs = []
            for _ in range(20):
                began = time.monotonic()
                screen.grab()
                grabs.append((time.monotonic() - began) * 1000)
            report["method"]["screenshot_ms_median"] = round(statistics.median(grabs), 1)
            for run in range(1, args.runs + 1):
                order = ("compass", "upstream") if run % 2 else ("upstream", "compass")
                for kind in order:
                    runs[kind].append(measure_one(kind, args, base, run, screen, wayland, bus))
        report["runs"] = runs
        report["summary"] = {
            kind: {
                "ready_ms": median_of(rs, "ready_ms"),
                "first_frame_ms": median_of(rs, "first_frame_ms"),
                "complete_ms": median_of(rs, "complete_ms"),
                "type_ms": median_of(rs, "type_ms"),
                "idle_rss_kib": median_of(rs, "idle", "rss_kib"),
                "idle_pss_kib": median_of(rs, "idle", "pss_kib"),
                "search_rss_kib": median_of(rs, "after_search", "rss_kib"),
                "search_pss_kib": median_of(rs, "after_search", "pss_kib"),
                "processes": median_of(rs, "idle", "processes"),
                "threads": median_of(rs, "idle", "threads"),
                "shared_objects": median_of(rs, "idle", "shared_objects"),
            } for kind, rs in runs.items()
        }
    finally:
        logs = out / "logs"
        logs.mkdir(exist_ok=True)
        for entry in [*base.glob("*.log"), *base.glob("*.png")]:
            shutil.copy2(entry, logs / entry.name)
        shutil.rmtree(base, ignore_errors=True)
    report["machine"]["loadavg_end"] = pathlib.Path("/proc/loadavg").read_text().split()[:3]
    (out / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report["summary"], indent=2))


def sha256(path):
    return hashlib.sha256(pathlib.Path(path).read_bytes()).hexdigest()


def cpu_model():
    for line in pathlib.Path("/proc/cpuinfo").read_text().splitlines():
        if line.startswith("model name"):
            return line.split(":", 1)[1].strip()
    return platform.processor()


def mem_total():
    return int(pathlib.Path("/proc/meminfo").read_text().split()[1])


def os_release():
    for line in pathlib.Path("/etc/os-release").read_text().splitlines():
        if line.startswith("PRETTY_NAME="):
            return line.split("=", 1)[1].strip('"')
    return None


if __name__ == "__main__":
    main()
