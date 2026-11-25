"""scripts/open_path_probe.py: its JSON fields, its edit/takeoff pieces on a
tiny document, and (opt-in, it opens a window) the real-window run."""
import importlib.util
import json
import os
import sys
from pathlib import Path

import pytest
from built_core import Matrix4 as QMatrix4x4, Vec3 as V

from built_core.group import Group
from built_core.history import History
from built_core.mesh import Mesh
from built_core.scene import Scene

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "open_path_probe.py"


def _probe():
    spec = importlib.util.spec_from_file_location("open_path_probe", SCRIPT)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def _box(x, s=1.0):
    m = Mesh()
    p = [V(x, 0, 0), V(x + s, 0, 0), V(x + s, s, 0), V(x, s, 0)]
    q = [V(c.x(), c.y(), s) for c in p]
    m.add_face(list(reversed(p)))
    m.add_face(q)
    for i in range(4):
        j = (i + 1) % 4
        m.add_face([p[i], p[j], q[j], q[i]])
    return m


def _scene():
    """A loose box, then an assembly of two leaves sharing one mesh and a
    third with its own."""
    scene = Scene()
    shared = _box(0)
    leaves = [Group(shared, name="shared a"), Group(shared, name="shared b"),
              Group(_box(4), name="own")]
    parent = Group(Mesh(), name="assembly")
    parent.children = leaves
    parent.xform = QMatrix4x4()
    for leaf in leaves:
        leaf.xform = QMatrix4x4()
    scene.groups += [Group(_box(8), name="loose"), parent]
    return scene


def test_result_has_every_field_and_writes_json(tmp_path):
    probe = _probe()
    doc = tmp_path / "tiny.igz"
    doc.write_bytes(b"x")
    result = probe.new_result(doc, {"edit": True, "takeoff": False})
    for name in probe.TIMING_FIELDS:
        assert name in result and result[name] is None
    for name in probe.INFO_FIELDS:
        assert name in result
    assert result["load_average"]["start"] and len(result["load_average"]["start"]) == 3
    assert set(result["modules"]) >= {"built_core", "built_render"}
    result["load"] = 1.23456789
    out = probe.write_result(result, tmp_path / "deep" / "probe.json")
    back = json.loads(out.read_text())
    assert back["load"] == 1.2346
    assert back["options"] == {"edit": True, "takeoff": False}
    assert probe.rss_peak_mb() > 10
    now = probe.rss_now_mb()
    assert now is None or now > 10


def test_edit_target_is_a_nested_unshared_leaf_and_the_command_moves_it():
    probe = _probe()
    scene = _scene()
    path = probe.pick_edit_target(scene)
    assert [g.name for g in path] == ["assembly", "own"]
    scene.begin_group_edit(path[0])
    scene.begin_group_edit(path[1])
    assert scene.edit_group is path[1]
    face = probe.edit_face(scene.mesh)
    before = [V(p) for p in face.vertices]
    normal = face.normal()
    History(scene).execute(probe.edit_command(face))
    moved = {(round(p.x(), 4), round(p.y(), 4), round(p.z(), 4))
             for p in (b + normal * 0.01 for b in before)}
    now = {(round(v.position.x(), 4), round(v.position.y(), 4), round(v.position.z(), 4))
           for v in scene.mesh.vertices}
    assert moved <= now


def test_cold_takeoff_reports_time_and_objects():
    probe = _probe()
    took, objects = probe.cold_takeoff(_scene())
    assert took >= 0 and objects == 0          # nothing tagged


def _can_open_a_window():
    if sys.platform != "darwin":
        return "the probe drives the Metal view (macOS only)"
    if os.environ.get("BUILT_PROBE_WINDOW") != "1":
        return "opens a real window on screen: set BUILT_PROBE_WINDOW=1"
    try:
        import built_render  # noqa: F401
    except ImportError:
        return "built_render is not built"
    return None


@pytest.mark.skipif(_can_open_a_window() is not None,
                    reason=_can_open_a_window() or "")
def test_real_window_run_writes_every_measurement(tmp_path):
    from formats.igz import save_scene
    doc = tmp_path / "tiny.igz"
    save_scene(_scene(), doc, backup=False)
    out = tmp_path / "probe.json"
    env = {k: v for k, v in os.environ.items() if k != "QT_QPA_PLATFORM"}
    from scripts.isolated_launch import run
    run([sys.executable, str(SCRIPT), str(doc), str(out), "--edit",
                    "--takeoff", "--timeout", "120"], cwd=ROOT, env=env,
                   timeout=180, check=True, capture_output=True)
    result = json.loads(out.read_text())
    for name in ("load", "pack", "upload", "first_frame", "snap_index_ready",
                 "max_main_thread_gap_ms", "rss_peak_mb", "rss_after_mb",
                 "first_edit", "takeoff"):
        assert result[name] is not None, name
    assert result["rust_engine"] in ("Rust", "Python")
    assert result["edit_target"]["path"] == ["assembly", "own"]
    assert not result["notes"]
