# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Whole-model switches, isolation and disclosure in the native Project tree."""
import pytest

from tests.native_tree import action, group_rows
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from tests.test_browser_layer_order import _container, _group
from tests.test_tree_links import _visible


@pytest.fixture
def model(native_host):
    inner = _group("Estructura")
    tops = [_container("Edificio", [inner]), _group("Casa"), _group("Muro")]
    native_host.viewport.scene.groups[:] = tops
    native_host.viewport.scene.version += 1
    return native_host, tops, inner


@pytest.mark.parametrize("verb,field", [("headereye", "hidden"), ("headerlock", "locked")])
def test_model_header_closes_then_opens_every_top_level_row(model, verb, field):
    host, tops, inner = model
    action(host, verb, "placements")
    assert all(getattr(g, field) for g in tops) and not getattr(inner, field)
    assert len(host.viewport.history.undo_stack) == 1
    action(host, verb, "placements")
    assert not any(getattr(g, field) for g in tops)
    assert host.viewport.history.undo() and all(getattr(g, field) for g in tops)


@pytest.mark.parametrize("verb,field", [("headereye", "hidden"), ("headerlock", "locked")])
def test_mixed_model_header_opens_all_and_undo_restores_individual_states(model, verb, field):
    host, tops, _ = model
    setattr(tops[1], field, True)
    action(host, verb, "placements")
    assert not any(getattr(g, field) for g in tops)
    assert host.viewport.history.undo()
    assert [getattr(g, field) for g in tops] == [False, True, False]


def test_nested_lock_survives_header_switches_and_the_open_edit_ends(model):
    host, tops, inner = model
    inner.locked = True
    host.viewport.begin_group_edit(tops[0])
    for expected in (True, False):
        action(host, "headerlock", "placements")
        assert tops[0].locked == expected and inner.locked
        assert host.viewport.scene.edit_group is None


def test_headers_respect_a_read_only_document(model):
    host, tops, _ = model
    host.viewport.scene.worksession_read_only = True
    action(host, "headerlock", "placements")
    action(host, "headereye", "tags")
    assert not any(g.locked for g in tops) and all(tag.visible for tag in host.viewport.scene.layers)
    assert not host.viewport.history.undo_stack


def test_isolating_and_leaving_restores_the_rendered_rows(model):
    host, tops, _ = model
    vp = host.viewport
    action(host, "isolaterows", "i:" + tops[1].uid)
    assert [_visible(vp.scene, g) for g in tops] == [False, True, False]
    assert "Isolate" in group_rows(host, tops[1])
    assert vp._isolate_base and not vp.history.undo_stack
    action(host, "showall", "")
    assert all(_visible(vp.scene, g) for g in tops) and not getattr(vp, "_isolate_base", None)


def test_show_all_restores_manual_hides_then_unhide_all_reaches_nested_rows(model):
    host, tops, inner = model
    vp = host.viewport
    inner.hidden = tops[2].hidden = True
    assert not getattr(vp, "_isolate_base", None)
    action(host, "isolaterows", "i:" + tops[1].uid)
    action(host, "showall", "")
    assert inner.hidden and tops[2].hidden and not tops[0].hidden
    host.window._on_unhide_all()
    assert not any(g.hidden for g in [*tops, inner])
    assert vp.history.undo() and inner.hidden and tops[2].hidden


def test_isolation_with_no_rows_explains_why(model, monkeypatch):
    host, tops, _ = model
    said = []
    monkeypatch.setattr(host.viewport, "flash_status", lambda msg, *a: said.append(msg))
    action(host, "isolaterows", "i:missing")
    assert said and all(_visible(host.viewport.scene, g) for g in tops)
    assert not getattr(host.viewport, "_isolate_base", None)


def test_disclosure_commands_recognise_manually_expanded_branches():
    from tests.test_project_drawing_filters import Project
    from tests.test_structure import model
    project = Project(scene=model()[0])
    project.rows()
    project.action("collapseall", "")
    assert "n:track" in project.action("toggle", "n:ride")
    assert "n:sections" in project.action("expandall", "")
    rows = project.action("collapseall", "")
    assert "n:ride" in rows and "n:track" not in rows
