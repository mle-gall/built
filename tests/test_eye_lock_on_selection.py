# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The eye and the padlock of one of several selected rows act on all of
them, as the material panel does. The press used to select that one row
first, so only it was hidden or locked; it now acts at once and leaves the
selection alone. Real mouse presses on the tree's viewport."""
from __future__ import annotations

import pytest

from ingetrazo_render import browser_selection
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from tests.native_tree import action
from tests.test_browser_layer_order import _group


@pytest.fixture
def model(native_host):
    groups = [_group(name) for name in ("A", "B", "C")]
    scene = native_host.viewport.scene
    scene.groups[:] = groups
    scene.version += 1
    return native_host, scene, groups


def press(host, group, verb):
    action(host, verb, "i:" + group.uid)


@pytest.mark.parametrize("verb,attr", [("roweye", "hidden"), ("rowlock", "locked")])
def test_on_a_selected_row_it_acts_on_every_selected_row(model, verb, attr):
    host, scene, (a, b, c) = model
    scene.select([a, b])
    press(host, b, verb)
    assert getattr(a, attr) and getattr(b, attr) and not getattr(c, attr)
    assert not scene.selection


@pytest.mark.parametrize("verb,attr", [("roweye", "hidden"), ("rowlock", "locked")])
def test_on_a_row_outside_the_selection_it_acts_on_that_row_alone(model, verb, attr):
    host, scene, (a, b, c) = model
    scene.select([a, b])
    press(host, c, verb)
    assert getattr(c, attr) and not getattr(a, attr) and not getattr(b, attr)
    assert browser_selection(scene) == {id(a), id(b)}


@pytest.mark.parametrize("verb,attr", [("roweye", "hidden"), ("rowlock", "locked")])
def test_hidden_or_locked_rows_selected_together_come_back_together(model, verb, attr):
    host, scene, (a, b, _) = model
    for g in (a, b):
        setattr(g, attr, True)
        press(host, g, "toggleitem")
    assert browser_selection(scene) == {id(a), id(b)} and not scene.selection
    press(host, b, verb)
    assert not getattr(a, attr) and not getattr(b, attr)
    assert set(scene.selection) == {a, b}


def test_one_press_is_one_undo_step(model):
    host, scene, (a, b, _) = model
    scene.select([a, b])
    press(host, a, "roweye")
    assert len(host.viewport.history.undo_stack) == 1
    assert host.viewport.history.undo() and not a.hidden and not b.hidden


@pytest.mark.parametrize("attr", ["hidden", "locked"])
def test_a_hidden_or_locked_row_clicked_stays_selected(model, attr):
    host, scene, (a, _, _) = model
    setattr(a, attr, True)
    press(host, a, "pickrow")
    assert browser_selection(scene) == {id(a)} and a not in scene.selection
    host.viewport.notify_scene_changed()
    assert browser_selection(scene) == {id(a)}


def test_rows_hidden_with_the_eye_stay_picked_and_come_back_selected(model):
    host, scene, (_, b, c) = model
    press(host, b, "pickrow")
    press(host, c, "toggleitem")
    press(host, b, "roweye")
    assert b.hidden and c.hidden and not scene.selection
    assert browser_selection(scene) == {id(b), id(c)}
    press(host, b, "roweye")
    assert not b.hidden and not c.hidden and set(scene.selection) == {b, c}


def test_a_selection_made_in_the_view_drops_the_kept_rows(model):
    host, scene, (a, _, c) = model
    a.hidden = True
    press(host, a, "pickrow")
    assert browser_selection(scene) == {id(a)}
    scene.select([c])
    assert browser_selection(scene) == {id(c)}
