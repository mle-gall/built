# SPDX-License-Identifier: GPL-3.0-or-later
"""The Browser's nonblocking rename preview and atomic document command."""
import json

import pytest
from core.group import Group
from ingetrazo_core import browser_rename
from tests.native_tree import action
from tests.shell_isolation import native_question_host, restore_shell_patches  # noqa: F401
from ingetrazo_render import chrome_forms


@pytest.fixture
def model(native_question_host):
    host = native_question_host
    scene = host.viewport.scene
    groups = [Group(name=n) for n in ("Puerta", "Muro", "Ventana")]
    scene.groups[:] = groups
    scene.version += 1
    return host, scene, groups


def question(host, scene, order=(), *, selected=False, query=""):
    payload = [id(scene), selected, query, [(g.uid, False) for g in order]]
    action(host, "batchrename", json.dumps(payload))
    return chrome_forms.current(host.window)


def names(form):
    return [new for _, new in form.model.preview()]


def test_numbered_names_and_base_guess(model):
    host, scene, groups = model
    form = question(host, scene)
    assert names(form) == ["W01", "W02", "W03"]
    form.press("cancel")
    for i, group in enumerate(groups):
        group.name = f"Muro 0{i+1}"
    form = question(host, scene)
    assert names(form) == ["Muro01", "Muro02", "Muro03"]
    form.receive("base", "Muro ")
    form.receive("start", "9")
    form.receive("digits", "3")
    assert names(form) == ["Muro 009", "Muro 010", "Muro 011"]
    for key, value in [("start", "-1"), ("start", "1.5"), ("digits", "0"), ("digits", "7")]:
        form.receive(key, value)
        assert form.model.error()
        form.press("ok")
        assert chrome_forms.current(host.window) is form
        form.receive(key, "2")


def test_case_sensitive_literal_replacements_and_unchanged_preview(model):
    host, scene, groups = model
    for g, name in zip(groups, ["Muro 1", "muro 2", "a.b"]):
        g.name = name
    form = question(host, scene)
    form.receive("mode", "1")
    assert form.model.error() and form.wire()[-1][-1][-1] & 128
    form.receive("find", "MURO")
    form.receive("replace", "Pared")
    assert form.model.preview() == [("Muro 1","Pared 1"), ("muro 2","Pared 2"), ("a.b","a.b")]
    assert form.wire()[-1][-1][-1] == 1
    form.receive("case", "1")
    form.receive("find", "Muro")
    assert names(form) == ["Pared 1", "muro 2", "a.b"]
    form.receive("find", ".")
    form.receive("replace", r"\1$")
    assert names(form)[-1] == r"a\1$b"


def test_empty_names_refused_and_cancel_does_not_edit(model):
    host, scene, groups = model
    form = question(host, scene)
    form.receive("mode", "1")
    form.receive("find", "Puerta")
    form.press("ok")
    assert form.error and not form.closed
    form.receive("mode", "2")
    form.receive("prefix", "N-")
    form.receive("suffix", "-01")
    assert names(form)[-1] == "N-Ventana-01"
    form.press("cancel")
    assert groups[-1].name == "Ventana" and not host.viewport.history.undo_stack


def test_tree_order_is_one_undo_and_redo(model):
    host, scene, (door, wall, window) = model
    scene.select([window, door, wall])
    form = question(host, scene, [wall, door, window], selected=True)
    assert [a for a, _ in form.model.preview()] == ["Muro", "Puerta", "Ventana"]
    form.press("ok")
    assert [door.name, wall.name, window.name] == ["W02","W01","W03"]
    assert len(host.viewport.history.undo_stack) == 1
    assert host.viewport.history.undo() and wall.name == "Muro"
    assert host.viewport.history.redo() and wall.name == "W01"


def test_header_uses_picked_or_all_matching_including_nested_rows(model):
    host, scene, groups = model
    other = Group(name="Muro norte")
    groups[0].children = [other]
    scene.select([groups[1], groups[2]])
    form = question(host, scene)  # header opened before the selection refresh
    assert [a for a, _ in form.model.preview()] == ["Muro", "Ventana"]
    form.press("cancel")
    scene.clear_selection()
    form = question(host, scene, [groups[1]], query="muro")
    assert [a for a, _ in form.model.preview()] == ["Muro","Muro norte"]
    form.press("cancel")
    assert browser_rename(scene, json.dumps([id(scene), False, "no matches", []])) is None
    assert browser_rename(scene, json.dumps([id(scene) + 1, False, "", []])) is None


@pytest.mark.parametrize("stale", ["removed","renamed","read-only","other document"])
def test_changed_targets_or_document_never_partly_apply(model, stale):
    host, scene, groups = model
    form = question(host, scene)
    if stale == "removed":
        scene.groups.pop()
    elif stale == "renamed":
        groups[-1].name = "External change"
    elif stale == "read-only":
        scene.worksession_read_only = True
    else:
        host.window.new_document()
    form.press("ok")
    assert groups[0].name == "Puerta" and not host.viewport.history.undo_stack


def test_single_item_uses_the_inline_editor_and_a_face_pick_never_renames_the_model(model, monkeypatch):
    from types import SimpleNamespace
    from ingetrazo_core import Vec3
    host, scene, groups = model
    scene.select([groups[1]])
    inline = []
    monkeypatch.setattr(host.view, "renderer", SimpleNamespace(
        ui_rename_browser_row=lambda row: inline.append(row) or True))
    assert question(host, scene, groups) is None
    assert inline == [groups[1].uid]
    face = scene.mesh.add_face([Vec3(), Vec3(1, 0, 0), Vec3(0, 1, 0)])
    scene.select([face])
    assert question(host, scene, groups) is None
    assert inline == [groups[1].uid]
