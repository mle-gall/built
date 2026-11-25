"""Measure a native window opening a document, editing nested geometry and computing takeoff.

    scripts/open_path_probe.py model.built /tmp/open.json --edit --takeoff
    scripts/open_path_probe.py --summarise /tmp/summary.json /tmp/open*.json

An isolated script plugin times the document reader, scene pack, GPU upload,
first presentation and inference index. A winit timer measures main-thread
gaps. Edits exist only in the private process and are never saved.
"""
from __future__ import annotations

import json
import os
import resource
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]

#: Every field the probe writes, so a reader (and the test) can rely on them.
TIMING_FIELDS = ("load", "pack", "upload", "first_frame", "snap_index_ready",
                 "snap_index_build", "max_main_thread_gap_ms",
                 "max_main_thread_gap_after_first_frame_ms", "rss_peak_mb",
                 "rss_after_mb", "first_edit", "takeoff")
INFO_FIELDS = ("probe_version", "file", "file_mb", "options", "git_sha",
               "git_dirty", "date", "machine", "load_average", "rust_engine",
               "modules", "window_size", "view_size", "events", "notes")
PROBE_VERSION = 2


# --- pieces the test exercises without a window ----------------------------

def rss_now_mb() -> float | None:
    """Resident memory now, in MB (``ps``; no psutil in the venv)."""
    try:
        out = subprocess.run(["ps", "-o", "rss=", "-p", str(os.getpid())],
                             capture_output=True, text=True, timeout=5)
        return round(int(out.stdout.split()[0]) / 1024, 1)
    except (OSError, ValueError, IndexError, subprocess.SubprocessError):
        return None


def rss_peak_mb() -> float:
    """Peak resident memory of this process, in MB (bytes on macOS)."""
    peak = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
    return round(peak / (2**20 if sys.platform == "darwin" else 1024), 1)


def _git(*args) -> str:
    try:
        return subprocess.run(["git", *args], cwd=REPO, capture_output=True,
                              text=True, timeout=10).stdout.strip()
    except (OSError, subprocess.SubprocessError):
        return ""


def module_paths() -> dict:
    """Where the native modules this run imports live (None: not importable)."""
    paths = {}
    for name in ("ingetrazo_core", "ingetrazo_render"):
        try:
            mod = __import__(name)
            paths[name] = getattr(mod, "__file__", None)
        except Exception as exc:  # noqa: BLE001 — record, never stop
            paths[name] = None
            paths[name + "_error"] = str(exc)
    return paths


def new_result(path: Path, options: dict) -> dict:
    """The result skeleton: every field present, timings None until measured."""
    path = Path(path)
    result = {name: None for name in TIMING_FIELDS}
    import platform
    result.update(
        probe_version=PROBE_VERSION,
        file=str(path),
        file_mb=round(path.stat().st_size / 2**20, 1) if path.exists() else None,
        options=dict(options),
        git_sha=_git("rev-parse", "HEAD"),
        git_dirty=bool(_git("status", "--porcelain", "--untracked-files=no")),
        date=time.strftime("%Y-%m-%dT%H:%M:%S%z"),
        machine={"platform": platform.platform(), "machine": platform.machine(),
                 "python": platform.python_version(), "cpus": os.cpu_count()},
        load_average={"start": list(os.getloadavg())},
        rust_engine=None,
        modules=module_paths(),
        window_size=None,
        view_size=None,
        events={},
        notes=[],
    )
    return result


def write_result(result: dict, out: Path) -> Path:
    """Write ``result`` as JSON (rounded floats), creating the folder."""
    def clean(value):
        if isinstance(value, float):
            return round(value, 4)
        if isinstance(value, dict):
            return {str(k): clean(v) for k, v in value.items()}
        if isinstance(value, (list, tuple)):
            return [clean(v) for v in value]
        return value
    out = Path(out)
    out.parent.mkdir(parents=True, exist_ok=True)
    tmp = out.with_suffix(out.suffix + ".tmp")
    tmp.write_text(json.dumps(clean(result), indent=2) + "\n")
    tmp.replace(out)
    return out


def pick_edit_target(scene, max_faces: int = 200):
    """The path (top-level group → leaf) to the first small leaf group nested
    at least one level deep (depth first, document order) whose mesh no
    other group shares, not hidden, locked or linked. None when there is
    none. Reads face counts without building deferred meshes."""
    from core.mesh import face_count
    from core import links
    counts: dict = {}
    stack = list(scene.groups)
    while stack:
        g = stack.pop()
        counts[id(g.mesh)] = counts.get(id(g.mesh), 0) + 1
        stack.extend(getattr(g, "children", None) or ())

    def usable(g):
        return not (g.hidden or g.locked or links.is_linked(g))

    def walk(groups, path):
        for g in groups:
            if not usable(g):
                continue
            here = path + [g]
            children = getattr(g, "children", None) or ()
            if children:
                found = walk(children, here)
                if found is not None:
                    return found
                continue
            if (len(here) >= 2 and counts.get(id(g.mesh), 0) == 1
                    and 4 <= face_count(g.mesh) <= max_faces):
                return here
        return None
    return walk(scene.groups, [])


def edit_face(mesh):
    """The first face of ``mesh`` with a usable normal (None: none)."""
    for face in mesh.faces:
        if len(face.loop) >= 3 and face._newell().length() > 1e-9:
            return face
    return None


def edit_command(face, distance: float = 0.01):
    """The Move tool's command for ``face`` moved ``distance`` along its normal."""
    from core.history import MoveVerticesCommand
    return MoveVerticesCommand(list(face.vertices), face.normal() * distance,
                               carry_textures=True)


def cold_takeoff(scene) -> tuple[float, int]:
    """(seconds, objects) of the BIM tray's takeoff with a fresh memo."""
    from core.bim import collect_objects
    start = time.perf_counter()
    objects = collect_objects(scene, {})
    return time.perf_counter() - start, len(objects)


# --- the real window --------------------------------------------------------

class Probe:
    def __init__(self, path: Path, out: Path, options: dict):
        self.path = path
        self.out = out
        self.options = options
        self.result = new_result(path, options)
        self.t_open = None
        self.t_first_frame = None
        self.t_index = None
        self.window = None
        self.calls: dict = {}
        self.gap_last = None
        self.gaps: list = []
        self.gap_timer = None
        self.set_scene_log: list = []
        self.timeline: list = []
        self.next_call = None
        self.frames: list = []
        self.pack_calls: list = []
        self.done = False
        self.lock = threading.Lock()

    def now(self) -> float:
        return time.perf_counter() - self.t_open if self.t_open is not None else None

    def event(self, name, value=None):
        t = self.now()
        self.result["events"][name] = round(t, 4) if value is None else value
        print(f"[probe] {name}: {self.result['events'][name]}", flush=True)

    # wrappers ---------------------------------------------------------------
    def install(self):
        from formats import igz
        import views.wgpu_view as wv
        import views.wgpu_inference as wi
        from ingetrazo_render import Host
        from ingetrazo_render.shell_views import ShellView
        import ingetrazo_core
        probe = self

        load_into = igz.load_into

        def timed_load(scene, path, *a, **k):
            start = time.perf_counter()
            try:
                return load_into(scene, path, *a, **k)
            finally:
                if probe.t_open is not None and probe.result["load"] is None \
                        and Path(path) == probe.path:
                    probe.result["load"] = time.perf_counter() - start
                    probe.event("load_done")
                    probe.result["rss_after_load_mb"] = rss_now_mb()
        igz.load_into = timed_load

        pack_scene = ingetrazo_core.pack_scene

        def timed_pack(scene, *a, **k):
            start = time.perf_counter()
            packed = pack_scene(scene, *a, **k)
            took = time.perf_counter() - start
            probe.pack_calls.append((start, took, len(packed.instances)))
            if (probe.t_open is not None and probe.result["pack"] is None
                    and k.get("progress") is not None):
                probe.result["pack"] = took
                probe.result["packed_instances"] = int(len(packed.instances))
                probe.event("pack_done")
            return packed
        wv.pack_scene = ingetrazo_core.pack_scene = timed_pack

        upload = ShellView._upload

        def timed_upload(view, *a, **k):
            start = time.perf_counter()
            packed = getattr(view, "packed", None)
            try:
                return upload(view, *a, **k)
            finally:
                if (probe.result["pack"] is not None and probe.result["upload"] is None
                        and packed is not None and len(packed.instances)
                        and getattr(view, "renderer", None) is not None):
                    probe.result["upload"] = time.perf_counter() - start
                    probe.event("upload_done")
        ShellView._upload = timed_upload

        set_scene = ShellView.set_scene

        def logged_set_scene(view, *a, **k):
            start = time.perf_counter()
            n_packs = len(probe.pack_calls)
            try:
                return set_scene(view, *a, **k)
            finally:
                probe.set_scene_log.append(
                    (start, time.perf_counter(), len(probe.pack_calls) > n_packs))
        ShellView.set_scene = logged_set_scene

        present = ShellView._present_frame

        def counted_present(view, *a, **k):
            out = present(view, *a, **k)
            if out is not None and view is probe.display():
                probe.on_frame()
            return out
        ShellView._present_frame = counted_present

        prepare = wi.prepare

        def timed_prepare(*a, **k):
            start = time.perf_counter()
            try:
                return prepare(*a, **k)
            finally:
                end = time.perf_counter()
                with probe.lock:
                    if probe.t_open is not None and probe.result["snap_index_build"] is None:
                        probe.result["snap_index_build"] = end - start
                        probe.result["events"]["snap_index_build_start"] = round(
                            start - probe.t_open, 4)
        wi.prepare = timed_prepare

        open_path = Host.open_path
        def timed_open(host, path):
            if Path(path).resolve() == probe.path and probe.t_open is None:
                probe.start(host.window)
            return open_path(host, path)
        Host.open_path = timed_open

    def start(self, window):
        self.window = window
        self.t_open = self.gap_last = time.perf_counter()
        self.gap_timer = window._host.timers.timer(self.tick, 10, True)
        self.gap_timer.start()

    def later(self, ms, fn):
        self.next_call = (time.perf_counter() + ms / 1000, fn)

    def display(self):
        doc = getattr(self.window, "_doc", None)
        return getattr(doc, "stand_in", None)

    def tick(self):
        t = time.perf_counter()
        if self.t_index is not None:
            if self.next_call and t >= self.next_call[0]:
                _, callback = self.next_call
                self.next_call = None
                callback()
            return
        gap = (t - self.gap_last) * 1000
        self.gap_last = t
        self.gaps.append((gap, t - self.t_open, self.t_first_frame is not None))
        display = self.display()
        if t - getattr(self, "_last_state", 0) >= 1.0 and len(self.timeline) < 600:
            # Once a second: is the view on screen and presenting?
            self._last_state = t
            doc = getattr(self.window, "_doc", None)
            self.timeline.append({
                "t": round(t - self.t_open, 2),
                "view_visible": bool(display is not None and display.isVisible()),
                "awaiting_picture": bool(getattr(doc, "_awaiting_picture", False)),
                "frames": len(self.frames)})
        if (self.t_first_frame is not None and self.t_index is None
                and getattr(display, "inference_index", None) is not None):
            self.t_index = t
            self.result["snap_index_ready"] = t - self.t_first_frame
            self.event("snap_index_ready")
            self.summarise_gaps()
            self.result["rss_after_mb"] = rss_now_mb()
            self.result["rss_peak_mb"] = rss_peak_mb()
            self.later(1000, self.after_index)

    def on_frame(self, *_):
        t = time.perf_counter()
        self.frames.append(t)
        if (self.t_open is not None and self.t_first_frame is None
                and self.result["upload"] is not None and self.window._doc.same_file(self.path)):
            self.t_first_frame = t
            self.result["first_frame"] = t - self.t_open
            self.event("first_presented")
            self.result["load_average"]["first_frame"] = list(os.getloadavg())

    def summarise_gaps(self):
        if not self.gaps:
            return
        self.result["max_main_thread_gap_ms"] = max(g for g, _, _ in self.gaps)
        after = [g for g, _, ff in self.gaps if ff]
        self.result["max_main_thread_gap_after_first_frame_ms"] = max(after) if after else None
        worst = sorted(self.gaps, reverse=True)[:5]
        self.result["top_gaps"] = [{"gap_ms": g, "ended_at": t, "after_first_frame": ff}
                                   for g, t, ff in worst]
        self.result["ticks"] = len(self.gaps)

    # after the index ---------------------------------------------------------
    def after_index(self):
        from core import _native
        self.result["rust_engine"] = _native.engine()
        display = self.display()
        if display is not None:
            self.result["view_size"] = [display.width(), display.height(),
                                        display.devicePixelRatioF()]
            self.result["renderer_adapter"] = getattr(
                getattr(display, "renderer", None), "adapter", None)
        self.result["window_size"] = [self.window._host.width, self.window._host.height]
        doc = getattr(self.window, "_doc", None)
        self.result["after_picture_jobs_left"] = len(getattr(doc, "_after_picture", []) or [])
        steps = []
        if self.options["edit"]:
            steps.append(self.run_edit)
        if self.options["takeoff"]:
            steps.append(self.run_takeoff)
        steps.append(self.finish)
        self.run_steps(steps)

    def run_steps(self, steps):
        if not steps:
            return
        step, rest = steps[0], steps[1:]
        step(lambda: self.later(200, lambda: self.run_steps(rest)))

    def wait_view(self, since, done, timeout=120.0):
        """Call ``done(t_frame, repacked)`` at the first frame presented
        after a ``set_scene`` that started after ``since`` has finished (or
        at the first frame after ``since`` when no ``set_scene`` came within
        two seconds)."""

        def poll():
            now = time.perf_counter()
            calls = [c for c in self.set_scene_log if c[0] >= since]
            if calls:
                end = calls[0][1]
                repacked = any(c[2] for c in calls)
                frames = [f for f in self.frames if f > end]
                if frames:
                    return done(frames[0], repacked, "set_scene")
            elif now - since > 2.0:
                frames = [f for f in self.frames if f > since]
                if frames:
                    return done(frames[0], False, "frame only")
            if now - since > timeout:
                return done(None, None, "timeout")
            self.later(5, poll)
        poll()

    def run_edit(self, next_step):
        vp = self.window.viewport
        scene = vp.scene
        start = time.perf_counter()
        target = pick_edit_target(scene)
        self.result["edit_target_search"] = time.perf_counter() - start
        if target is None:
            self.result["notes"].append("edit: no small unshared nested leaf group found")
            return next_step()
        path = target
        from core.mesh import face_count
        self.result["edit_target"] = {
            "path": [g.name for g in path], "depth": len(path),
            "faces": face_count(path[-1].mesh)}
        t_enter = time.perf_counter()
        for g in path:
            vp.begin_group_edit(g)
        self.result["enter_group_call"] = time.perf_counter() - t_enter
        if scene.edit_group is not path[-1]:
            self.result["notes"].append(
                f"edit: begin_group_edit did not open {path[-1].name!r}")
            return next_step()

        def entered(t_frame, repacked, how):
            if t_frame is not None:
                self.result["enter_group"] = t_frame - t_enter
            self.result["enter_group_path"] = how
            self.result["enter_group_repacked"] = repacked
            self.later(500, edit)

        def edit():
            from core.history import CommandRejected
            # The open group's mesh (a component instance edits a world copy).
            face = edit_face(scene.mesh)
            if face is None:
                self.result["notes"].append("edit: the open group has no usable face")
                return next_step()
            self.result["edit_target"]["face_vertices"] = len(face.loop)
            cmd = edit_command(face)
            t_edit = time.perf_counter()
            try:
                vp.history.execute(cmd)
            except CommandRejected as exc:
                self.result["notes"].append(f"edit: command refused: {exc}")
                return next_step()
            self.result["edit_command"] = time.perf_counter() - t_edit
            vp.update()

            def shown(t_frame, repacked, how):
                if t_frame is not None:
                    self.result["first_edit"] = t_frame - t_edit
                self.result["first_edit_path"] = how
                self.result["first_edit_repacked"] = repacked
                calls = [c for c in self.set_scene_log if c[0] >= t_edit]
                if calls:
                    self.result["first_edit_set_scene"] = calls[0][1] - calls[0][0]
                print(f"[probe] first_edit: {self.result['first_edit']}", flush=True)
                next_step()
            self.wait_view(t_edit, shown)
        self.wait_view(t_enter, entered)

    def run_takeoff(self, next_step):
        took, n = cold_takeoff(self.window.viewport.scene)
        self.result["takeoff"] = took
        self.result["takeoff_objects"] = n
        print(f"[probe] takeoff: {took:.2f} s, {n} objects", flush=True)
        next_step()

    def finish(self, _next=None):
        # rss_peak_mb stays the open path's (set when the index was ready).
        self.result["rss_peak_end_mb"] = rss_peak_mb()
        self.result["rss_end_mb"] = rss_now_mb()
        self.result["load_average"]["end"] = list(os.getloadavg())
        self.result["frames_presented"] = len(self.frames)
        self.write_and_exit(0)

    def write_and_exit(self, code):
        if self.done:
            return
        self.done = True
        if self.result["max_main_thread_gap_ms"] is None:
            self.summarise_gaps()
        self.result["frames_presented"] = len(self.frames)
        self.result["timeline"] = self.timeline
        write_result(self.result, self.out)
        print(f"[probe] wrote {self.out}", flush=True)
        os._exit(code)

    def watchdog(self, seconds):
        def fire():
            self.result["notes"].append(f"watchdog: stopped after {seconds} s")
            if self.result["rss_peak_mb"] is None:
                self.result["rss_peak_mb"] = rss_peak_mb()
            self.write_and_exit(2)
        timer = threading.Timer(seconds, fire)
        timer.daemon = True
        timer.start()


#: What ``--summarise`` takes the median of (beside TIMING_FIELDS).
EXTRA_FIELDS = ("enter_group", "first_edit_set_scene", "edit_command",
                "rss_after_load_mb", "rss_peak_end_mb", "packed_instances",
                "takeoff_objects")


def summarise(paths) -> dict:
    """Median and spread of several runs, grouped by file."""
    import statistics
    by_file: dict = {}
    for path in paths:
        run = json.loads(Path(path).read_text())
        by_file.setdefault(run["file"], []).append((str(path), run))
    out = {"probe_version": PROBE_VERSION, "files": {}}
    for name, runs in by_file.items():
        entry = {"runs": [p for p, _ in runs], "median": {}, "min": {}, "max": {},
                 "load_average_1min": [r["load_average"]["start"][0] for _, r in runs],
                 "git_sha": sorted({r["git_sha"] for _, r in runs}),
                 "rust_engine": sorted({str(r["rust_engine"]) for _, r in runs}),
                 "notes": sorted({n for _, r in runs for n in r["notes"]})}
        for field in TIMING_FIELDS + EXTRA_FIELDS:
            values = [r.get(field) for _, r in runs if r.get(field) is not None]
            if values:
                entry["median"][field] = statistics.median(values)
                entry["min"][field] = min(values)
                entry["max"][field] = max(values)
        first = runs[0][1]
        for key in ("file_mb", "machine", "modules", "window_size", "view_size",
                    "renderer_adapter", "edit_target", "options"):
            entry[key] = first.get(key)
        out["files"][name] = entry
    return out


def parse_args(argv):
    import argparse
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("file", type=Path)
    parser.add_argument("out", type=Path)
    parser.add_argument("--edit", action="store_true")
    parser.add_argument("--takeoff", action="store_true")
    parser.add_argument("--perf", action="store_true",
                        help="INGETRAZO_PERF=1 telemetry into <out>.perf.log")
    parser.add_argument("--size", default="1600x1000",
                        help="main window size, WxH (default 1600x1000)")
    parser.add_argument("--no-on-top", action="store_true",
                        help=argparse.SUPPRESS)
    parser.add_argument("--timeout", type=float, default=900.0,
                        help="write what was measured and stop after this many seconds")
    return parser.parse_args(argv)


def main(argv=None) -> int:
    argv = sys.argv[1:] if argv is None else argv
    if argv and argv[0] == "--summarise":
        # --summarise <out.json> <run.json>...: medians per file.
        write_result(summarise(argv[2:]), Path(argv[1]))
        print(Path(argv[1]).read_text())
        return 0
    args = parse_args(argv)
    path = args.file.expanduser().resolve()
    w, h = (int(v) for v in args.size.lower().split("x"))
    options = {"edit": args.edit, "takeoff": args.takeoff, "perf": args.perf,
               "size": [w, h]}
    from scripts.isolated_launch import run
    with tempfile.TemporaryDirectory(prefix="built-open-probe-") as folder:
        profile = Path(folder) / "profile"
        plugins = profile / "data" / "ingetrazo" / "plugins"
        plugins.mkdir(parents=True)
        (plugins / "open_probe.py").write_text(
            "from scripts.open_path_probe import install_probe as setup\n")
        report = Path(folder) / "probe.json"
        env = dict(os.environ, BUILT_OPEN_PROBE=json.dumps(
            [str(path), str(report), options, args.timeout]))
        if args.perf:
            env.update(INGETRAZO_PERF="1", INGETRAZO_PERF_LOG=str(args.out.resolve().with_suffix(".perf.log")))
        result = run([REPO / "rust/built_app/target/release/built", "--new-instance",
                      "--root", REPO, "--venv", REPO / ".venv", "--size", args.size, path],
                     settings=profile, env=env, timeout=args.timeout + 30)
        if report.is_file():
            write_result(json.loads(report.read_text()), args.out.resolve())
        else:
            print("Native probe exited without a measurement", file=sys.stderr)
            return result.returncode or 1
        return result.returncode


def install_probe(app):
    path, out, options, timeout = json.loads(os.environ["BUILT_OPEN_PROBE"])
    probe = Probe(Path(path), Path(out), options)
    probe.install()
    probe.watchdog(timeout)


if __name__ == "__main__":
    sys.path.insert(0, str(REPO))
    sys.exit(main())
