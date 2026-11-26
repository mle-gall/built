# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Render with Blender (issue #181): finding Blender, the job, the child's
environment and the progress — and, where this machine has Blender, one
real render end to end."""
from __future__ import annotations

import json
import py_compile
from pathlib import Path
import struct
import subprocess

import pytest
from built_core import Vec3 as QVector3D
from tests.shell_isolation import restore_shell_patches  # noqa: F401

from built_core import render_blender as rb  # noqa: E402
from built_core.camera import OrbitCamera  # noqa: E402
from built_core.scene import Scene  # noqa: E402


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


def test_a_packaged_built_does_not_hand_blender_its_libraries():
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
    # The sandbox's /app/built/… path does not exist for Blender on the
    # host: «Python file "/app/built/resources/blender/render_scene.py"
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
    assert rb.work_base() == tmp_path / ".cache" / "Built" / "render"


@pytest.mark.parametrize("line, expected", [
    ("BUILT stats Remaining: 00:03.05 | Mem: 77M | Sample 16/64", 0.25),
    ("BUILT stats Rendering 32 / 64 samples", 0.5),
    ("BUILT stats Mem: 0M | Synchronizing object | Built", None),
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


@pytest.mark.slow
@pytest.mark.blender
def test_a_real_render_through_the_users_blender(tmp_path):
    scene = _box_scene()
    job = rb.write_job(scene, _camera(scene), tmp_path, engine="cycles",
                       quality=0, width=160, height=90, sample_override=8, preview=True)
    found = rb.find_blender()
    result = subprocess.run(rb.command(found, job), capture_output=True, text=True,
                            env=rb.clean_env(), timeout=600)
    assert result.returncode == 0, (result.stdout + result.stderr)[-4000:]
    assert "BUILT preview" in result.stdout and (tmp_path / "render.png.preview.png").is_file()
    assert "BUILT done" in result.stdout
    assert _png_size(tmp_path / "render.png") == (160, 90)


def test_the_panel_knows_where_blender_is(native_host, monkeypatch):
    from plugins._blender_panel import create_panel
    from views.extension_api import ExtensionApp

    found = rb.BlenderFound(["/x/blender"], "/x/blender")
    monkeypatch.setattr(rb, "find_blender", lambda saved: found)
    monkeypatch.setattr(rb, "in_snap", lambda: False)
    panel = create_panel(ExtensionApp(native_host.window, "render_blender"), lambda: None)
    controls = {item["id"]: item for section in panel.sections() for item in section["items"]}
    assert controls["availability"]["available"] and controls["render"]["enabled"]
    assert "binary" not in controls and "engine" not in controls

    monkeypatch.setattr(rb, "in_snap", lambda: True)
    panel.on_open()
    controls = {item["id"]: item for section in panel.sections() for item in section["items"]}
    assert not controls["render"]["enabled"]
    assert "Snap" in controls["activity"]["label"]
    panel.event("render", "invoke", None)
    assert panel.proc is None
