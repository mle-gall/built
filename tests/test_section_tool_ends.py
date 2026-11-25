# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Issue #62 (@pacaeiro): the Section Plane tool ends after one plane, and
the name prompt can be switched off."""
from __future__ import annotations


from ingetrazo_core import ScreenPoint as QPointF, Input as Qt
from ingetrazo_core import Vec3 as QVector3D


from core.history import History  # noqa: E402
from core.scene import Scene  # noqa: E402
from tools.base import ToolContext  # noqa: E402
from tools.section import SectionPlaneTool  # noqa: E402


class _Win:
    def __init__(self):
        self.activated: list = []
        self.prompted: list = []

    def _activate_tool(self, key):
        self.activated.append(key)

    def prompt_section_name(self, plane):
        self.prompted.append(plane)


class _Vp:
    def __init__(self, scene, win):
        self.scene = scene
        self.history = History(scene)
        self._win = win
        self.camera = None

    def window(self):
        return self._win

    def update(self):
        pass

    def flash_status(self, *a, **k):
        pass


def test_one_plane_then_back_to_select():
    scene = Scene()
    win = _Win()
    vp = _Vp(scene, win)
    tool = SectionPlaneTool()
    tool._normal = QVector3D(0, 0, 1)
    tool.on_click(ToolContext(viewport=vp, world=QVector3D(0, 0, 1),
                              screen=QPointF(0, 0), modifiers=Qt.NoModifier,
                              snap=None))
    assert len(scene.section_planes) == 1
    assert win.prompted and win.activated == ["select"]


from tests.shell_isolation import native_window, restore_shell_patches  # noqa: F401


def test_the_name_prompt_can_be_switched_off(monkeypatch, native_window):
    from ingetrazo_core.settings import settings as current_settings
    from ingetrazo_render import chrome_forms
    from core.history import PlaceSectionPlaneCommand
    from ingetrazo_core.section import SectionPlane

    forms = []
    monkeypatch.setattr(chrome_forms, "show", lambda _, form: forms.append(form))
    win = native_window
    settings = current_settings()
    settings.setValue("section/ask_name", "1")
    vp = win.viewport
    plane = SectionPlane(QVector3D(0, 0, 0), QVector3D(0, 0, 1),
                         name="Sección 1", symbol="A")
    vp.history.execute(PlaceSectionPlaneCommand(plane))
    win.prompt_section_name(plane)
    form = forms.pop()
    form.receive("name", "Cancelled")
    form.receive("skip", "1")
    form.press("cancel")
    assert plane.name == "Sección 1" and settings.value("section/ask_name") == "1"

    win.prompt_section_name(plane)
    form = forms.pop()
    form.receive("name", "North")
    form.receive("symbol", "三四五六")
    form.receive("skip", "1")
    form.press("ok")
    assert (plane.name, plane.symbol) == ("North", "三四五")
    assert settings.value("section/ask_name") == "0"
    win.prompt_section_name(plane)
    assert not forms
    win.prompt_section_name(plane, automatic=False)
    assert all(row[0] != "skip" for row in forms.pop().model.rows())
    vp.history.undo()
    assert (plane.name, plane.symbol) == ("Sección 1", "A")


# ---- #62, second round -------------------------------------------------

def test_the_floating_plane_wears_the_inference_colours():
    from core.snap import AXIS_COLORS, COLOR_REFERENCE
    tool = SectionPlaneTool()
    for axis, n in (("x", QVector3D(1, 0, 0)), ("y", QVector3D(0, -1, 0)),
                    ("z", QVector3D(0, 0, 1))):
        tool._normal = n
        assert tool.wireframe_color[:3] == tuple(AXIS_COLORS[axis][:3])
    tool._normal = QVector3D(1, 1, 0).normalized()     # a face off the axes
    assert tool.wireframe_color[:3] == tuple(COLOR_REFERENCE)


def test_placing_a_plane_switches_the_cuts_back_on():
    scene = Scene()
    scene.show_section_cuts = False
    scene.show_section_planes = False
    win = _Win()
    synced = []
    win._sync_section_menu = lambda: synced.append(True)
    vp = _Vp(scene, win)
    tool = SectionPlaneTool()
    tool.on_click(ToolContext(viewport=vp, world=QVector3D(0, 0, 1),
                              screen=QPointF(0, 0), modifiers=Qt.NoModifier,
                              snap=None))
    assert scene.show_section_cuts and scene.show_section_planes
    assert synced                                     # the menu checkmarks follow
