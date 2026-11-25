# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Render with Blender (issue #181): finding Blender, the job, the child's
environment and the progress — and, where this machine has Blender, one
real render end to end."""
from __future__ import annotations

import json
import os
import py_compile
from pathlib import Path
import re
import shutil
import signal
import struct
import subprocess
import sys
import time

import pytest
from ingetrazo_core import Vec3 as QVector3D
from tests.shell_isolation import restore_shell_patches  # noqa: F401

from core import render_blender as rb  # noqa: E402
from core.camera import OrbitCamera  # noqa: E402
from core.scene import Scene  # noqa: E402


def _box_scene():
    scene = Scene()
    m = scene.mesh
    V = QVector3D
    pts = [V(0, 0, 0), V(2, 0, 0), V(2, 2, 0), V(0, 2, 0),
           V(0, 0, 2), V(2, 0, 2), V(2, 2, 2), V(0, 2, 2)]
    for loop in ((0, 3, 2, 1), (4, 5, 6, 7), (0, 1, 5, 4),
                 (1, 2, 6, 5), (2, 3, 7, 6), (3, 0, 4, 7)):
        m.add_face([pts[i] for i in loop])
    return scene


def _camera(scene):
    cam = OrbitCamera()
    cam.set_aspect(320, 180)
    cam.yaw, cam.pitch = -1.0, 0.4
    lo, hi = scene.bounds()
    cam.fit_to(lo, hi)
    return cam


def test_the_blender_script_is_valid_python():
    py_compile.compile(str(rb.render_script()), doraise=True)


def test_a_packaged_ingetrazo_does_not_hand_blender_its_libraries():
    env = rb.clean_env({"LD_LIBRARY_PATH": "/tmp/_MEIxyz",
                        "LD_LIBRARY_PATH_ORIG": "/usr/local/lib",
                        "PYTHONHOME": "/tmp/_MEIxyz", "HOME": "/home/x",
                        "QT_PLUGIN_PATH": "/tmp/_MEIxyz/qt"})
    assert env["LD_LIBRARY_PATH"] == "/usr/local/lib"
    assert "PYTHONHOME" not in env and "QT_PLUGIN_PATH" not in env
    assert env["HOME"] == "/home/x"


def test_a_blender_picked_by_hand_wins(tmp_path, monkeypatch):
    exe = tmp_path / "blender"
    exe.write_text("")
    monkeypatch.setattr(rb, "in_flatpak", lambda: False)
    found = rb.find_blender(str(exe))
    assert found.command == [str(exe)]


def test_inside_the_flatpak_blender_runs_on_the_host(monkeypatch):
    monkeypatch.setattr(rb, "in_flatpak", lambda: True)
    answers = {"sh": "/usr/bin/blender\n"}
    monkeypatch.setattr(rb, "_host", lambda args, timeout=10.0:
                        answers.get(args[0]))
    found = rb.find_blender()
    assert found.command == ["flatpak-spawn", "--host", "/usr/bin/blender"]
    # No system Blender, but Blender's own Flatpak on the host:
    answers.clear()
    answers["flatpak"] = "org.blender.Blender\n"
    found = rb.find_blender()
    assert found.command[:4] == ["flatpak-spawn", "--host", "flatpak", "run"]
    assert found.command[-1] == rb.BLENDER_FLATPAK_ID
    # The permission not granted: nothing answers, nothing found.
    answers.clear()
    assert rb.find_blender() is None


def test_the_command_runs_our_script_on_the_job(tmp_path):
    found = rb.BlenderFound(["/opt/blender/blender"], "x")
    argv = rb.command(found, tmp_path / "job.json")
    assert argv[:3] == ["/opt/blender/blender", "-b", "--factory-startup"]
    assert argv[argv.index("--python") + 1] == str(tmp_path / "render_scene.py")
    assert argv[-2:] == ["--", str(tmp_path / "job.json")]


def test_under_flatpak_the_script_travels_next_to_the_job(tmp_path,
                                                         monkeypatch):
    # The sandbox's /app/ingetrazo/… path does not exist for Blender on the
    # host: «Python file "/app/ingetrazo/resources/blender/render_scene.py"
    # could not be opened» (Marco, 0.5.6.1 Flatpak).
    monkeypatch.setattr(rb, "in_flatpak", lambda: True)
    found = rb.BlenderFound(["flatpak-spawn", "--host", "blender"], "x")
    job = tmp_path / "job.json"
    argv = rb.command(found, job)
    script = argv[argv.index("--python") + 1]
    assert script == str(tmp_path / "render_scene.py")
    assert (tmp_path / "render_scene.py").read_bytes() == \
        rb.render_script().read_bytes()


def test_under_flatpak_renders_go_where_the_host_sees_them(tmp_path,
                                                          monkeypatch):
    monkeypatch.setattr(rb, "in_flatpak", lambda: True)
    monkeypatch.setattr(Path, "home", classmethod(lambda cls: tmp_path))
    assert rb.work_base() == tmp_path / ".cache" / "IngeTrazo" / "render"


@pytest.mark.parametrize("line, expected", [
    ("INGETRAZO stats Remaining: 00:03.05 | Mem: 77M | Sample 16/64", 0.25),
    ("INGETRAZO stats Rendering 32 / 64 samples", 0.5),
    ("INGETRAZO stats Mem: 0M | Synchronizing object | IngeTrazo", None),
])
def test_progress_is_read_from_both_engines(line, expected):
    assert rb.progress_of(line) == expected


def test_the_job_carries_the_view_the_sun_and_the_model(tmp_path):
    scene = _box_scene()
    cam = _camera(scene)
    job = json.loads(rb.write_job(scene, cam, tmp_path, engine="cycles",
                                  quality=2, width=640, height=360,
                                  keep_blend=True).read_text())
    assert (tmp_path / "model.glb").stat().st_size > 0
    assert job["samples"] == rb.SAMPLES["cycles"][2]
    assert job["blend"].endswith("render.blend")
    eye = cam.eye()
    assert job["camera"]["eye"] == pytest.approx([eye.x(), eye.y(), eye.z()])
    assert job["camera"]["perspective"] is True
    assert job["sun"] is None or len(job["sun"]) == 3
    assert job["ground"]["z"] <= 0.0
    with pytest.raises(ValueError):
        rb.write_job(scene, cam, tmp_path, engine="povray")


def _png_size(path):
    with open(path, "rb") as fh:
        head = fh.read(24)
    assert head[:8] == b"\x89PNG\r\n\x1a\n"
    return struct.unpack(">II", head[16:24])


_KERNEL_SYMBOL = "ccl::MetalKernelPipeline::compile()"
# An Objective-C exception prints numbered frames. The malloc abort only
# leaves the symbolized stack in macOS's diagnostic report.
_KERNEL_FRAME = re.compile(
    r"^\s*\d+\s+\S+\s+0x[0-9a-fA-F]+\s+"
    r"(?:ccl::MetalKernelPipeline::compile\(\)|_+ZN3ccl19MetalKernelPipeline7compileEv)"
    r"(?:\s+\+\s+\d+)?\s*$", re.MULTILINE)


def _reported_compiler_abort(directory, pid, started):
    for path in directory.glob("Blender-*.ips"):
        try:
            if path.stat().st_mtime < started:
                continue
            with path.open() as report:
                header = json.loads(report.readline())
                if header.get("app_version") != "5.2.3":
                    continue
                crash = json.load(report)
        except (OSError, ValueError):
            continue  # ReportCrash may still be writing this report.
        if (crash.get("pid") != pid or crash.get("procName") != "Blender"
                or crash.get("exception", {}).get("signal") != "SIGABRT"):
            continue
        stacks = [t.get("frames", []) for t in crash.get("threads", []) if t.get("triggered")]
        stacks.append(crash.get("lastExceptionBacktrace", []))
        return any(frame.get("symbol") == _KERNEL_SYMBOL for stack in stacks for frame in stack)
    return False


def _crash_reports():
    import pwd
    return Path(pwd.getpwuid(os.getuid()).pw_dir) / "Library/Logs/DiagnosticReports"


def _cycles_metal_kernel_abort(code, stdout, stderr, pid, started):
    if (sys.platform != "darwin" or code != -signal.SIGABRT
            or "Blender 5.2.3 LTS " not in stdout
            or "INGETRAZO device METAL" not in stdout or "INGETRAZO engine CYCLES" not in stdout):
        return False
    if _KERNEL_FRAME.search(stderr):
        return True
    # macOS writes the report to the login user's Library even when the
    # test launches Blender with an isolated HOME.
    directory = _crash_reports()
    deadline = time.monotonic() + 3
    while not _reported_compiler_abort(directory, pid, started):
        if time.monotonic() >= deadline:
            return False
        time.sleep(0.1)
    return True


def test_only_the_known_macos_kernel_compiler_abort_is_skippable(monkeypatch, tmp_path):
    monkeypatch.setattr(sys, "platform", "darwin")
    monkeypatch.setattr(sys.modules[__name__], "_crash_reports", lambda: tmp_path)
    monkeypatch.setattr(time, "monotonic", iter(range(0, 100, 4)).__next__)
    stdout = "Blender 5.2.3 LTS (hash e13b728c6880)\nINGETRAZO device METAL\nINGETRAZO engine CYCLES"
    frame = "8   Blender    0x0000000103ab0000 ccl::MetalKernelPipeline::compile() + 42"

    def matches(output, trace, code=-signal.SIGABRT):
        return _cycles_metal_kernel_abort(code, output, trace, 123, 0)

    assert matches(stdout, frame)
    assert not matches(stdout, frame.replace("compile()", "load()"))
    assert not matches(stdout, "log message mentioning " + _KERNEL_SYMBOL)
    assert not matches(stdout.replace("5.2.3", "5.2.4"), frame)
    assert not matches(stdout.replace("METAL", "CPU"), frame)
    assert not matches(stdout, frame, 1)


def test_malloc_abort_requires_the_crashed_blender_thread(tmp_path):
    report = tmp_path / "Blender-test.ips"
    header = {"app_version": "5.2.3"}
    frames = [{"symbol": "ccl::path_cache_get(...)"}, {"symbol": _KERNEL_SYMBOL}]
    thread = {"triggered": True, "frames": frames}
    crash = {"pid": 789, "procName": "Blender", "exception": {"signal": "SIGABRT"}, "threads": [thread]}

    def write():
        report.write_text(json.dumps(header) + "\n" + json.dumps(crash))

    write()
    assert _reported_compiler_abort(tmp_path, 789, 0)
    assert not _reported_compiler_abort(tmp_path, 123, 0)
    assert not _reported_compiler_abort(tmp_path, 789, report.stat().st_mtime + 1)
    thread["triggered"] = False
    write()
    assert not _reported_compiler_abort(tmp_path, 789, 0)


@pytest.mark.slow
@pytest.mark.skipif(shutil.which("blender") is None,
                    reason="no Blender on this machine")
def test_a_real_render_through_the_users_blender(tmp_path):
    scene = _box_scene()
    job = rb.write_job(scene, _camera(scene), tmp_path, engine="cycles",
                       quality=0, width=160, height=90)
    found = rb.find_blender()
    started = time.time()
    with subprocess.Popen(rb.command(found, job), stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                          text=True, env=rb.clean_env()) as child:
        try:
            stdout, stderr = child.communicate(timeout=600)
        except subprocess.TimeoutExpired:
            child.kill()
            child.communicate()
            raise
    if _cycles_metal_kernel_abort(child.returncode, stdout, stderr, child.pid, started):
        pytest.skip("Blender 5.2.3 aborted inside the Cycles Metal kernel compiler")
    assert child.returncode == 0, (stdout + stderr)[-4000:]
    assert "INGETRAZO done" in stdout
    assert _png_size(tmp_path / "render.png") == (160, 90)


def test_the_panel_knows_where_blender_is(native_host, monkeypatch):
    from plugins._blender_panel import create_panel
    from views.extension_api import ExtensionApp

    found = rb.BlenderFound(["/x/blender"], "/x/blender")
    monkeypatch.setattr(rb, "find_blender", lambda saved: found)
    monkeypatch.setattr(rb, "in_snap", lambda: False)
    panel = create_panel(ExtensionApp(native_host.window, "render_blender"), lambda: None)
    controls = {item["id"]: item for section in panel.sections() for item in section["items"]}
    assert controls["binary"]["value"] == found.where and controls["render"]["enabled"]

    monkeypatch.setattr(rb, "in_snap", lambda: True)
    panel.event("binary", "commit", found.where)
    controls = {item["id"]: item for section in panel.sections() for item in section["items"]}
    assert not controls["render"]["enabled"]
    assert "Snap" in controls["activity"]["label"]
    panel.event("render", "invoke", None)
    assert panel.proc is None
