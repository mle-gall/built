# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Camera ▸ Component Edit: Hide Rest of Model (Alt+Q) and Hide
Similar Components (Alt+W).

Hide Rest toggles the existing «rest of model while editing» mode between
``hide`` and whatever it was before; Hide Similar takes the other copies of
the component being edited out of the frame — and out of the pick.
"""
from __future__ import annotations

from built_core import Matrix4 as QMatrix4x4, Vec3 as QVector3D
from tests.shell_isolation import restore_shell_patches  # noqa: F401

from core.group import Group
from core.mesh import Mesh
from core.scene import Scene


def V(x, y, z=0.0):
    return QVector3D(float(x), float(y), float(z))


def _instance(proto, dx):
    g = Group(proto, name="Silla")
    m = QMatrix4x4()
    m.translate(dx, 0, 0)
    g.xform = m
    return g


def _three_chairs():
    """Two copies of one definition, and an unrelated component."""
    scene = Scene()
    proto = Mesh()
    proto.add_face([V(0, 0), V(1, 0), V(1, 1), V(0, 1)])
    other = Mesh()
    other.add_face([V(0, 0), V(2, 0), V(2, 2), V(0, 2)])
    a, b = _instance(proto, 0), _instance(proto, 5)
    c = _instance(other, 10)
    scene.groups.extend([a, b, c])
    return scene, a, b, c


def test_hide_similar_hides_only_the_siblings_while_editing():
    scene, a, b, c = _three_chairs()
    scene.hide_similar_components = True
    assert scene.entity_visible(b), "nothing is hidden at the root"
    scene.begin_group_edit(a)
    assert not scene.entity_visible(b)
    assert not scene.entity_selectable(b)
    assert scene.entity_visible(c), "another definition stays"
    assert scene.entity_visible(a)
    scene.end_group_edit()
    assert scene.entity_visible(b)


def test_hide_similar_off_leaves_the_siblings():
    scene, a, b, _c = _three_chairs()
    scene.begin_group_edit(a)
    assert scene.entity_visible(b)
    scene.end_group_edit()


def test_a_classic_group_has_no_similar():
    scene = Scene()
    m = Mesh()
    m.add_face([V(0, 0), V(1, 0), V(1, 1), V(0, 1)])
    g = Group(m, name="Muro")
    scene.groups.append(g)
    scene.hide_similar_components = True
    scene.begin_group_edit(g)
    assert scene.edit_definition() is None
    scene.end_group_edit()


def test_the_shortcuts_are_alt_q_and_alt_w(native_host):
    from tests.test_shortcut_editor import keyboard
    sheet = keyboard(native_host.window)
    assert sheet._action("sc:menu:hide_rest").portable_keys() == ["Alt+Q"]
    assert sheet._action("sc:menu:hide_similar").portable_keys() == ["Alt+W"]


def test_hide_rest_toggles_back_to_the_previous_mode(native_host):
    vp = native_host.viewport
    vp.set_edit_rest_mode("normal")
    assert native_host.menu("hide_rest")
    assert vp.edit_rest_mode == "hide"
    assert native_host.menu("hide_rest")
    assert vp.edit_rest_mode == "normal"


def test_fade_slider_applies_immediately_and_follows_the_viewport(native_host):
    from built_render.settings_sheet import Env, SettingsSheet
    win = native_host.window
    env = Env(win)
    def slider():
        return next(row for _, _, group in SettingsSheet().groups(env, "display")
                    for row in group if row.id == "rest_fade")
    assert callable(win.viewport._edit_rest_fade)
    assert native_host.menu("rest_fade")
    assert slider().on("enabled", env)
    slider().put(env, 38)
    assert abs(win.viewport.edit_rest_fade - .38) < 1e-9
    assert slider().fmt(slider().value(env)) == "38 %"
    assert native_host.menu("rest_hide")
    assert not slider().on("enabled", env)


def test_hide_similar_action_flips_the_scene_flag(native_host):
    scene = native_host.viewport.scene
    before = scene.hide_similar_components
    assert native_host.menu("hide_similar")
    assert scene.hide_similar_components is (not before)
