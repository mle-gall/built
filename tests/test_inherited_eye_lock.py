# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""What a hidden or locked group holds is hidden or locked too, and the
Browser now says so: its eye or padlock shows closed but faded, «Hidden by
«Edificio»», «Locked by «Edificio»». Nothing inside is changed — each row
keeps its own state, so unlocking the group gives every row back as it
was, a row locked on purpose still locked."""
from __future__ import annotations

from built_core import Matrix4 as QMatrix4x4, Vec3 as V

from built_core.group import Group
from built_core.i18n import tr
from built_core.mesh import Mesh
import pytest
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from tests.native_tree import action
from tests.test_project_drawing_filters import Project


def _g(name, kids=()):
    m = Mesh()
    m.add_face([V(0, 0, 0), V(1, 0, 0), V(1, 1, 0), V(0, 1, 0)])
    g = Group(m, name=name)
    if kids:
        g.xform = QMatrix4x4()
        g.component = False
        g.children = list(kids)
    return g


def _model():
    cocina = _g("Cocina")
    planta = _g("Planta baja", [cocina])
    estructura = _g("Estructura")
    edificio = _g("Edificio", [planta, estructura])
    return edificio, planta, cocina, estructura


@pytest.fixture
def model(native_host):
    groups = _model()
    scene = native_host.viewport.scene
    scene.groups[:] = [groups[0]]
    scene.version += 1
    return native_host, groups


def rows(host):
    p = Project(scene=host.viewport.scene)
    p.action("filter", "*")
    return {text[1]: (text, flags) for text, flags in p.view.renderer.calls["ui_set_tree"][0]}


def press(host, group, verb):
    action(host, verb, "i:" + group.uid)


def test_the_nearest_closed_group_around_each_one(model):
    _, (house, floor, room, wall) = model
    house.locked = True
    floor.hidden = True
    assert house._closed_ancestors() == (None, None)
    assert floor._closed_ancestors() == (None, house.name)
    assert room._closed_ancestors() == (floor.name, house.name)
    assert wall._closed_ancestors() == (None, house.name)


def test_the_rows_inside_a_locked_group_show_a_faded_padlock(model):
    host, (house, floor, room, wall) = model
    house.locked = wall.locked = True
    snapshot = rows(host)
    for child in (floor, room):
        text, flags = snapshot[child.name]
        assert flags[6] == 2 and text[-1] == tr("Locked by «{name}»", name=house.name)
        assert not child.locked
    assert snapshot[wall.name][1][6] == 1


def test_unlocking_the_group_gives_every_row_back_as_it_was(model):
    host, (house, floor, room, wall) = model
    wall.locked = True
    press(host, house, "rowlock")
    assert house.locked
    press(host, house, "rowlock")
    assert not house.locked and wall.locked
    assert rows(host)[floor.name][1][6] == 0 and not room.locked


def test_the_rows_inside_a_hidden_group_show_a_faded_eye_and_grey_names(model):
    host, (house, _, room, _) = model
    house.hidden = True
    text, flags = rows(host)[room.name]
    assert flags[5] == 2 and text[-2] == tr("Hidden by «{name}»", name=house.name)
    assert not room.hidden


@pytest.mark.parametrize("attr,verb,msg", [
    ("locked", "rowlock", "Locked by «{name}»: unlock it first"),
    ("hidden", "roweye", "Hidden by «{name}»: show it first"),
])
def test_pressing_an_inherited_control_explains_without_editing(model, attr, verb, msg, monkeypatch):
    host, (house, _, child, _) = model
    setattr(house, attr, True)
    said = []
    monkeypatch.setattr(host.viewport, "flash_status", lambda text, *_: said.append(text))
    press(host, child, verb)
    assert not getattr(child, attr) and not host.viewport.history.undo_stack
    assert said == [tr(msg, name=house.name)]


def test_a_row_locked_on_its_own_inside_unlocks_on_its_own(model):
    host, (house, _, _, wall) = model
    house.locked = wall.locked = True
    press(host, wall, "rowlock")
    assert not wall.locked
    assert rows(host)[wall.name][1][6] == 2
