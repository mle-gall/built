"""Run the inspector and menu verbs on the receiving native document."""
import math

from ingetrazo_core import Vec3
from tests.shell_isolation import native_window, restore_shell_patches  # noqa: F401
from views import chrome_forms, view_commands
from ingetrazo_render import context_menu_build, context_menu_run


def invoke(win, verb):
    menu = context_menu_build(win, win.view)
    assert menu.find(verb) is not None, menu.ids()
    assert context_menu_run(win, win.view, menu.kind, verb, menu.sel, facts=menu.facts)


def test_reference_image_forms_keep_aspect_undo_and_reject_stale_answers(native_window, monkeypatch):
    from core.image_plane import ImagePlane
    win = native_window
    vp = win.viewport
    image = ImagePlane("synthetic.png", Vec3(), Vec3(4, 0, 0), Vec3(0, 2, 0), aspect=0.5)
    vp.scene.image_planes.append(image)
    vp.scene.select([image])
    answers = []
    monkeypatch.setattr(chrome_forms, "ask_number", lambda *a, done, **kw: answers.append(done))
    invoke(win, "imgsize")
    answers.pop()(8.0)
    assert (image.width(), image.height()) == (8.0, 4.0)
    invoke(win, "imgopacity")
    answers.pop()(25)
    assert image.opacity == 0.25
    assert vp.history.undo() and image.opacity == 1.0
    invoke(win, "imgsize")
    win.new_document()
    answers.pop()(12.0)
    assert image.width() == 8.0 and not win.viewport.history.undo_stack


def test_section_and_view_commands_change_the_camera(native_window):
    from core.section import SectionPlane
    win = native_window
    vp = win.viewport
    plane = SectionPlane(Vec3(2, 3, 4), Vec3(0, 1, 0))
    vp.scene.section_planes.append(plane)
    vp.scene.select([plane])
    invoke(win, "secalign")
    assert vp.camera.yaw == math.pi / 2 and vp.camera.target == plane.point
    invoke(win, "secreverse")
    assert plane.normal.y() == -1 and vp.history.undo() and plane.normal.y() == 1
    for action in ["view_top", "view_front", "back_edges", "hidden_geometry", "section_cuts", "shadows"]:
        assert view_commands.run(win, action)
    style = win.active_style()
    assert view_commands.run(win, "xray") and win.active_style().face_mode == "xray"
    assert view_commands.run(win, "xray") and win.active_style().face_mode == style.face_mode


def test_surface_rebuild_uses_its_own_datum(native_window, monkeypatch):
    from georef import dem, surface
    from georef.datum import SceneDatum
    from georef.geopath import GeoPath

    class Changed:
        def connect(self, done):
            self.done = done

    class Sampler:
        def __init__(self, datum):
            self.datum = datum
            self.changed = Changed()
            self.areas = []

        def ensure_area(self, *bounds):
            self.areas.append(bounds)

    monkeypatch.setattr(dem, "DEMSampler", Sampler)
    win = native_window
    vp = win.viewport
    scene = vp.scene
    calls = []
    monkeypatch.setattr(surface, "build_surface", lambda path, sampler, datum: calls.append(datum) or [1])
    scene.georef = SceneDatum(45, 2)
    path = GeoPath([Vec3(), Vec3(2, 0, 0), Vec3(2, 2, 0)])
    scene.geo_paths.append(path)
    scene.select([path])
    invoke(win, "surf:draped")
    sampler = vp._surface_sampler
    assert sampler.areas and path.closed and path._surface_tris == [1]
    sampler.changed.done()
    assert calls == [scene.georef, scene.georef]
    scene.georef = SceneDatum(44, 1)
    sampler.changed.done()
    assert len(calls) == 2
    invoke(win, "surf:none")
    assert path.surface is None and path._surface_tris is None
