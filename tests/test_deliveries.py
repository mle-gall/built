"""Named deliveries through the protocol, dock and one-off export."""
import json
import time
from types import SimpleNamespace
from unittest.mock import Mock

import pytest
from core import deliveries
from core.camera import OrbitCamera
from core.group import Group
from core.history import History
from core.protocol.commands import run
from core.scene import Scene
from formats import igz
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from tests.test_ifc_export import _tagged_wall_scene
import ingetrazo_render as dock
from app_shell import modal


def add(h, path, **fields):
    record = dict(name="Handover", format=path.suffix[1:], destination=str(path))
    return run(h, "create_delivery", delivery={**record, **fields})["key"]


@pytest.mark.parametrize("fmt", deliveries.FORMATS)
def test_delivery_lifecycle(fmt, tmp_path, request):
    scene = _tagged_wall_scene()
    ctx = {}
    if fmt == "pdf":
        from tests.test_outliner import _sheets_model
        from views.wgpu_sheets import composer_for
        win = request.getfixturevalue("native_host").window
        scene = win.viewport.scene
        sheets = _sheets_model(win)
        for sheet in sheets:
            sheet.frames.clear()
        ctx["pdf_writer"] = composer_for(win).export_all_pdf
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


class DeliveryWindow(SimpleNamespace):
    """A weak-referenceable owner for the native timer queue."""


def drain(timers):
    deadline = time.monotonic() + 5
    while (delay := timers.next_ms()) is not None:
        assert time.monotonic() + delay / 1000 < deadline
        time.sleep(delay / 1000)
        timers.run_due()


@pytest.fixture
def next_turn(monkeypatch):
    from core import input_types
    from ingetrazo_render import WindowTimers, chrome_dialogs
    timers = WindowTimers()
    monkeypatch.setattr(input_types, "NATIVE", True)
    monkeypatch.setattr(chrome_dialogs, "in_shell", lambda: True)
    return SimpleNamespace(timers=timers)


def test_queue_keep_cancel(tmp_path, next_turn):
    scene = _tagged_wall_scene()
    vp = SimpleNamespace(history=History(scene), scene=scene, camera=OrbitCamera())
    win = DeliveryWindow(_host=next_turn, viewport=vp)
    timers = next_turn.timers
    path = tmp_path / "client.glb"
    form = dock.delivery_export_form(win, one_off=True)
    form.receive("format", str(form.field("format").keys().index("glb")))
    form.set("destination", str(path))
    form.press("keep")
    key, record = next(iter(deliveries.records(scene).items()))
    cancelled = tmp_path / "cancel.glb"
    job = dock.enqueue_delivery(win, None, {**record, "destination": str(cancelled)})
    job.cancel()
    timers.run_due()
    assert win._chrome_jobs[0].phase == "Writing"
    drain(timers)
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


def test_checks_and_one_off_export(tmp_path, native_host):
    from views import wgpu_ui
    win = native_host.window
    scene = win.viewport.scene
    scene.groups[:] = [Group(name="Loose")]
    scene.version += 1
    add(win.viewport.history, tmp_path / "handover.ifc")
    from tests.test_tree_links import _View
    view = _View()
    view._ui_lens = 2
    wgpu_ui._tree(view, win)
    panel, blob = view.renderer.calls["ui_panel_set"]
    readiness = json.loads(blob)
    assert panel == "readiness" and (readiness["ready"], readiness["total"]) == (0, 1)
    assert readiness["checks"]["*"]["unfiled/"][0] == "Not filed"
    from ingetrazo_render import chrome_sheet
    native_host.menu("delivery_export")
    assert chrome_sheet.current(win).id == "export"
    chrome_sheet.close(win)


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


def test_running_export_observes_cancellation(tmp_path, monkeypatch, next_turn):
    from core import progress
    from formats import dxf_out
    s = _tagged_wall_scene()
    vp = SimpleNamespace(history=History(s), scene=s, camera=OrbitCamera())
    win = DeliveryWindow(_host=next_turn, viewport=vp)
    timers = next_turn.timers
    monkeypatch.setattr(progress, "REPORT_EVERY", 0)
    target = tmp_path / "plan.dxf"
    target.write_text("previous")
    key = add(vp.history, target)
    job = dock.enqueue_delivery(win, key, deliveries.records(s)[key])
    monkeypatch.setattr(modal, "tick", job.cancel)

    def writer(path, *unused):
        progress.tick(0.5)
        pytest.fail("Cancelled before publishing")

    monkeypatch.setattr(dxf_out, "save_dxf_layers", writer)
    drain(timers)
    assert job.cancelled and not win._chrome_jobs
    assert target.read_text() == "previous" and deliveries.records(s)[key]["last_run"] is None


@pytest.mark.parametrize("one_off", [True, False], ids=["export", "delivery"])
def test_svg_sheet_previews_the_page_it_writes(one_off, tmp_path, monkeypatch, next_turn, native_host):
    from ingetrazo_render import chrome_sheet as sheet

    scene = _tagged_wall_scene()
    camera = OrbitCamera()
    camera.perspective = False
    viewport = SimpleNamespace(scene=scene, camera=camera, history=History(scene))
    view = native_host.view
    renderer = Mock()
    monkeypatch.setattr(view, "renderer", renderer)
    monkeypatch.setattr(view, "width", lambda: 960)
    monkeypatch.setattr(view, "height", lambda: 640)
    monkeypatch.setattr(view, "_ui_attached", True, raising=False)
    win = DeliveryWindow(_host=next_turn, viewport=viewport, _doc=SimpleNamespace(stand_in=view))
    timers = next_turn.timers
    monkeypatch.setattr(sheet, "_arm_watchdog", lambda *args: None)

    form = dock.delivery_export_form(win, one_off=one_off)
    sid = "export" if one_off else "delivery"

    def act(*parts):
        sheet.before_frame(view, win)
        sheet.action(win, sheet.SEP.join([sid, *parts]))
        sheet.before_frame(view, win)

    target = tmp_path / "wall.svg"
    act("v", "format", str(deliveries.FORMATS.index("svg")))
    act("v", "destination", str(target))
    act("v", "size", "2")
    act("v", "paper_preset", "1")
    act("v", "fill", "3")
    act("v", "svg_preview", "1")
    renderer.ui_set_icons.assert_not_called()
    assert not target.exists()
    drain(timers)
    sheet.before_frame(view, win)
    block = json.loads(renderer.ui_set_large_sheet.call_args.args[0])["detail"]["blocks"][0]
    assert (block["kind"], block["text"]) == ("svg_preview", "Thirds")
    icons, = renderer.ui_set_icons.call_args.args
    assert [icon[0] for icon in icons] == ["delivery_svg_preview"]

    act("v", "svg_guides", "4")
    assert timers.next_ms() is None
    act("svg_frame", "0.1", "-0.2", "2")
    assert timers.next_ms() is not None
    act("verb", "export" if one_off else "save")
    assert form.closed and not sheet.shown(win)
    drain(timers)
    if one_off:
        assert target.is_file() and not deliveries.records(scene)
        options = dict(paper=[297, 210], frame=[-0.1, -0.2, 2], scale=100,
                       faces=dict(materials=True, textures=False, as_view=True, shadows=False))
        record = dict(format="svg", scope="whole", options=options)
    else:
        record, = deliveries.records(scene).values()
        assert not target.exists() and "svg_guides" not in record["options"]
        assert record["options"]["frame"] == [-0.1, -0.2, 2]
        list(deliveries.write_steps(scene, record, camera=camera))
    text = deliveries.preview_svg(scene, record, camera=camera)
    assert text == target.read_text() and "<path" in text
    from views.svg_export import preview_pixels
    w, h, rgba = preview_pixels(text)
    assert (w, h) == (800, 566) and len(rgba) == w * h * 4
