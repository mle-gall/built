"""Named deliveries through the protocol, dock and one-off export."""
from types import SimpleNamespace

import pytest
from core import deliveries
from core.camera import OrbitCamera
from core.group import Group
from core.history import History
from core.protocol.commands import run
from core.scene import Scene
from formats import igz
from tests.test_ifc_export import _tagged_wall_scene
from views import wgpu_deliveries as dock


def add(h, path, **fields):
    record = dict(name="Handover", format=path.suffix[1:], destination=str(path))
    return run(h, "create_delivery", delivery={**record, **fields})["key"]


@pytest.mark.parametrize("fmt", deliveries.FORMATS)
def test_delivery_lifecycle(fmt, tmp_path):
    scene = _tagged_wall_scene()
    ctx = {}
    if fmt == "pdf":
        from tests.test_outliner import _window_with, _sheets_model
        win, scene, _ = _window_with()
        sheets = _sheets_model(win)
        for sheet in sheets:
            sheet.frames.clear()
        ctx["pdf_writer"] = win._ensure_composer().export_all_pdf
    h = History(scene)
    path = tmp_path / f"export.{fmt}"
    key = add(h, path, scope="sheets" if fmt == "pdf" else "whole")
    run(h, "edit_delivery", key=key, changes={"name": "Plans"})
    assert deliveries.records(scene)[key]["name"] == "Plans"
    h.undo()
    assert deliveries.records(scene)[key]["name"] == "Handover"
    assert run(h, "run_delivery", key=key, **ctx)["ok"]
    assert path.stat().st_size and deliveries.records(scene)[key]["last_run"]
    assert run(h, "delete_delivery", key=key)["ok"]
    assert not deliveries.records(scene)
    h.undo()
    assert key in deliveries.records(scene)


@pytest.mark.parametrize("ext", ["built", "igz"])
def test_reopen(tmp_path, ext):
    scene = _tagged_wall_scene()
    add(History(scene), tmp_path / "plan.svg")
    path = tmp_path / ("plans." + ext)
    igz.save_scene(scene, path)
    reopened = Scene()
    igz.load_into(reopened, path)
    assert deliveries.records(reopened) == deliveries.records(scene)


def test_queue_keep_cancel(tmp_path, monkeypatch):
    from views import chrome_forms
    scene = _tagged_wall_scene()
    vp = SimpleNamespace(history=History(scene), scene=scene, camera=OrbitCamera())
    win = SimpleNamespace(viewport=vp)
    monkeypatch.setattr(chrome_forms, "show", lambda *_: None)
    ticks = []
    monkeypatch.setattr(dock.QTimer, "singleShot", lambda _, fn: ticks.append(fn))
    path = tmp_path / "client.glb"
    form = dock.export_form(win, one_off=True)
    form.receive("format", str(form.field("format").keys().index("glb")))
    form.set("destination", str(path))
    form.press("keep")
    key, record = next(iter(deliveries.records(scene).items()))
    cancelled = tmp_path / "cancel.glb"
    job = dock.enqueue(win, None, {**record, "destination": str(cancelled)})
    job.cancel()
    ticks.pop(0)()
    assert win._chrome_jobs[0].phase == "Writing"
    while ticks:
        ticks.pop(0)()
    assert not win._chrome_jobs and not cancelled.exists()
    assert path.read_bytes()[:4] == b"glTF" and deliveries.records(scene)[key]["last_run"]
    assert win._delivery_result[1] == tmp_path


def test_scope_node(tmp_path):
    from core import bim_registry
    scene = Scene()
    parent, child, outside = [Group(name=n) for n in ("A", "B", "Other")]
    mesh = _tagged_wall_scene().mesh
    child.mesh = outside.mesh = mesh
    parent.children = [child]
    scene.groups = [parent, outside]
    registry = bim_registry.get(scene)
    registry["structure"] = {"nodes": {"n": {"name": "A", "order": 0, "parent": None}}}
    scene.plugin_data["bim_registry"] = registry
    child.ext = {"structure": {"node": "n"}}
    projected = deliveries.scoped(scene, {"scope": "node", "node": "n"})
    assert set(projected.groups_by_uid()) == {parent.uid, child.uid}
    assert not projected.groups[0].mesh.faces
    assert projected.groups[0].children[0].mesh is mesh


def test_selected_and_visible_preserve_source(tmp_path):
    from formats.obj import save_obj
    scene = _tagged_wall_scene()
    original = list(scene.mesh.faces)
    scene.selection = [original[0]]
    original[-1].attrs["hidden"] = True
    for scope, count in [("whole", 6), ("visible", 5), ("selection", 1)]:
        model = deliveries.scoped(scene, {"scope": scope})
        assert len(model.mesh.faces) == count
        save_obj(model, tmp_path / (scope + ".obj"))
    assert list(scene.mesh.faces) == original and original[-1].attrs["hidden"]


def test_scenes_cancel_keeps_previous_files(tmp_path):
    from core.saved_views import SavedView
    s = _tagged_wall_scene()
    cam = OrbitCamera()
    cam.perspective = False
    s.saved_views = [SavedView.capture(name, s, cam) for name in ("First", "Second")]
    h = History(s)
    path = tmp_path / "plan.dxf"
    key = add(h, path, scope="scenes")
    first = tmp_path / "plan-01.dxf"
    first.write_text("prior")
    steps = deliveries.write_steps(s, deliveries.records(s)[key])
    assert next(steps) == (0, "First")
    assert next(steps) == (0.5, "Second")
    steps.close()
    assert first.read_text() == "prior" and not list(tmp_path.glob("plan-02*"))
    run(h, "run_delivery", key=key)
    assert all((tmp_path / f"plan-{i:02}.dxf").read_text().endswith("EOF\n") for i in (1, 2))


def test_checks_and_one_off_export(tmp_path, monkeypatch):
    from tests.test_outliner import _window_with
    from views import wgpu_ui
    win, scene, _ = _window_with(Group(name="Loose"))
    add(win.viewport.history, tmp_path / "handover.ifc")
    got = {}
    class Renderer:
        def ui_set_inspector(self, title, sections):
            got.update(sections)
        def ui_set_tree(self, rows, count):
            assert rows
        def ui_set_colset(self, colset):
            assert colset == 2
        def ui_set_mode(self, mode):
            assert mode == ""
    view = SimpleNamespace(renderer=Renderer())
    dock.dock(view, scene, wgpu_ui._tree_model(view, scene, scene.content_version))
    assert ("unfiled object", "1") in got["Pre-run checks"]
    actions = win._file_actions()
    assert any(a.text() == "Export…" for a in actions if hasattr(a, "text"))
    assert any(a.title() == "Export" for a in actions if hasattr(a, "title"))


def test_problem_checks_cover_filed_geometry():
    from core import bim_registry
    from views.wgpu_ui import _problem_rules
    s = Scene()
    group = Group(name="Unclassified wall")
    mesh = _tagged_wall_scene().mesh
    face = mesh.faces[0]
    group.mesh.add_face(face.vertices)
    group.ext = {"structure": {"node": "walls"}}
    s.groups = [group]
    reg = bim_registry.get(s)
    nodes = {"walls": {"name": "Walls", "order": 0, "parent": None, "recipe": {"class": "IfcWall"}}}
    reg["structure"] = {"nodes": nodes}
    rules = _problem_rules(s, reg, nodes, {"walls": nodes["walls"]["recipe"]}, [])
    by_id = {key: groups for _, key, _, groups in rules}
    assert by_id["solid"] == by_id["class"] == [group]


def test_running_export_observes_cancellation(tmp_path, monkeypatch):
    from core import progress
    from formats import dxf_out
    from PySide6.QtWidgets import QApplication
    s = _tagged_wall_scene()
    vp = SimpleNamespace(history=History(s), scene=s, camera=OrbitCamera())
    win = SimpleNamespace(viewport=vp)
    callbacks = []
    monkeypatch.setattr(dock.QTimer, "singleShot", lambda _, fn: callbacks.append(fn))
    monkeypatch.setattr(progress, "REPORT_EVERY", 0)
    target = tmp_path / "plan.dxf"
    target.write_text("previous")
    key = add(vp.history, target)
    job = dock.enqueue(win, key, deliveries.records(s)[key])
    monkeypatch.setattr(QApplication, "processEvents", job.cancel)

    def writer(path, *unused):
        progress.tick(0.5)
        pytest.fail("Cancelled before publishing")

    monkeypatch.setattr(dxf_out, "save_dxf_layers", writer)
    while callbacks:
        callbacks.pop(0)()
    assert job.cancelled and not win._chrome_jobs
    assert target.read_text() == "previous" and deliveries.records(s)[key]["last_run"] is None
