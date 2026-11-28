# SPDX-License-Identifier: GPL-3.0-or-later
"""The Sketch workspace's Release (leave the sketch and keep the result: one
History step that commits the drawing and gives each user its own frozen
copy) and the Library's section thumbnails (rendered from the section,
cached by its drawing, rendered again after an edit)."""
import json
from types import SimpleNamespace

import pytest

from built_core.history import History
from built_core.protocol import commands
from built_core.scene import Scene
from tests.test_sketch_rows import apply, extent, member, rect, rows_of, sketch


def two_members(h, s):
    apply(h, "sketch_commit", row=s)
    apply(h, "create_sweeps", sweeps=[member(dict(sketch=s["uid"])),
                                      member(dict(sketch=s["uid"]), ((0, 1, 0), (2, 1, 0)))])
    return h.scene.groups


def test_release_commits_the_drawing_and_gives_each_member_its_own_copy_in_one_step():
    scene = Scene()
    h = History(scene)
    s = sketch(rect(200, 100))
    a, b = two_members(h, s)
    assert extent(a) == pytest.approx([2, 0.2, 0.1])
    steps = len(h.undo_stack)
    # Drawn wider in the workspace, then Release instead of Done.
    reply = apply(h, "sketch_commit", row=sketch(rect(300, 100)), release=True)
    assert reply["released"] == 2 and reply["uid"] == s["uid"]
    assert len(h.undo_stack) == steps + 1
    copies = rows_of(scene, "sketch")
    assert len(copies) == 2 and s["uid"] not in [c["uid"] for c in copies]
    assert all(c["frozen"] is True and c["state"] == "ok" for c in copies)
    sweeps = rows_of(scene, "sweep")
    assert sorted(r["profile"]["sketch"] for r in sweeps) == sorted(c["uid"] for c in copies)
    assert all(r["state"] == "ok" for r in sweeps)
    # Both members took the edit before they were let go.
    assert extent(a) == pytest.approx([2, 0.3, 0.1]) and extent(b) == pytest.approx([2, 0.3, 0.1])
    # One of them edited afterwards moves alone.
    mine = next(c for c in copies if c["uid"] == next(r for r in sweeps if r["owner"] == a.uid)["profile"]["sketch"])
    apply(h, "sketch_commit", row=dict(mine, curves=rect(120, 100)))
    assert extent(a) == pytest.approx([2, 0.12, 0.1]) and extent(b) == pytest.approx([2, 0.3, 0.1])
    apply(h, "undo")
    # One undo for the release: the shared sketch and the old size are back.
    apply(h, "undo")
    (row,) = rows_of(scene, "sketch")
    assert row["uid"] == s["uid"] and "frozen" not in row
    assert extent(a) == pytest.approx([2, 0.2, 0.1]) and extent(b) == pytest.approx([2, 0.2, 0.1])
    apply(h, "redo")
    assert len(rows_of(scene, "sketch")) == 2 and extent(b) == pytest.approx([2, 0.3, 0.1])


def test_release_refuses_a_sketch_nothing_uses_and_an_open_drawing():
    scene = Scene()
    h = History(scene)
    s = sketch(rect(200, 100))
    apply(h, "sketch_commit", row=s)
    reply = commands.run(h, "sketch_commit", row=s, release=True)
    assert not reply["ok"] and "Nothing uses this sketch" in reply["rejected"]
    (row,) = rows_of(scene, "sketch")
    assert row["uid"] == s["uid"], "refused: the sketch is still there"
    apply(h, "create_sweeps", sweeps=[member(dict(sketch=s["uid"]))])
    (g,) = scene.groups
    open_drawing = sketch(rect(200, 100)[:3])
    reply = commands.run(h, "sketch_commit", row=open_drawing, release=True)
    assert not reply["ok"] and "Close the drawing" in reply["rejected"]
    assert rows_of(scene, "sketch")[0]["state"] == "ok" and extent(g) == pytest.approx([2, 0.2, 0.1])


def test_release_of_a_catalogue_member_s_drawing_adopts_and_freezes_it_in_one_step():
    from built_core import command_protocol
    scene = Scene()
    h = History(scene)
    apply(h, "create_sweeps", sweeps=[member(dict(library="en/ipe240"))])
    (g,) = scene.groups
    (row,) = rows_of(scene, "sweep")
    size = extent(g)
    drawn = command_protocol.profile_sketch(rows_of(scene, "sweep")[0]["profile"])
    steps = len(h.undo_stack)
    reply = apply(h, "sketch_commit", row=dict(drawn, uid="5ce7c40000000019"), adopt=[row["uid"]], release=True)
    assert reply["released"] == 1 and len(h.undo_stack) == steps + 1
    (copy,) = rows_of(scene, "sketch")
    assert copy["frozen"] is True and copy["uid"] != "5ce7c40000000019"
    assert rows_of(scene, "sweep")[0]["profile"]["sketch"] == copy["uid"]
    assert extent(g) == pytest.approx(size)
    apply(h, "undo")
    assert rows_of(scene, "sketch") == [] and rows_of(scene, "sweep")[0]["profile"]["library"] == "en/ipe240"


def test_release_freezes_a_section_cut_s_drawing():
    from built_core.solids import solid_volume
    scene = Scene()
    h = History(scene)
    apply(h, "create_sweeps", sweeps=[member(dict(library="basic/timber"))])
    (g,) = scene.groups
    before = solid_volume(g)
    notch = sketch(rect(20, 400), uid="5ce7c4000000001a", use="section", name="Notch")
    for curve in notch["curves"]:
        for point in ("a", "b"):
            x, y = curve[point]
            curve[point] = [x + 1000, y - 200]
    apply(h, "sketch_commit", row=notch)
    apply(h, "live_set", row=dict(kind="cut", owner=g.uid, uid="c07000000000001a",
                                  cutter={"section": notch["uid"]}, operation="subtract"))
    cut = solid_volume(g)
    assert cut < before
    reply = apply(h, "sketch_commit", row=notch, release=True)
    assert reply["released"] == 1
    (copy,) = rows_of(scene, "sketch")
    assert copy["frozen"] is True and copy["use"] == "section"
    assert rows_of(scene, "cut")[0]["cutter"] == {"section": copy["uid"]}
    assert solid_volume(g) == pytest.approx(cut)


def store(root, kind, name, curves, rev=1):
    """A saved Library item in My library's folder (docs/library.md)."""
    folder = root / "data" / "Built" / "Library" / kind
    folder.mkdir(parents=True, exist_ok=True)
    body = {"use": kind[:-1], "name": name, "curves": curves}
    data = {"format": "built.profile", "v": 1, "name": name, "use": kind[:-1], "sketch": body}
    entry = {"format": "built.library", "v": 1, "kind": kind, "id": name.lower(), "rev": rev, "name": name,
             "description": "", "tags": [], "saved": rev, "data": data}
    (folder / f"{name.lower()}.json").write_text(json.dumps(entry), encoding="utf-8")
    return body


def test_sections_show_a_thumbnail_of_their_drawing_cached_and_refreshed_on_edit(tmp_path, monkeypatch):
    from built_render.wgpu_library import SectionsProvider, sketch_thumbnail

    monkeypatch.setenv("BUILT_SETTINGS_DIR", str(tmp_path))
    first = store(tmp_path, "sections", "Notch", rect(40, 30))
    scene = Scene()
    h = History(scene)
    apply(h, "sketch_commit", row=sketch(rect(10, 60), uid="5ce7c400000000b1", use="section", name="Cope"))
    listed = lambda: {i["name"]: i for i in SectionsProvider().items(SimpleNamespace(scene=scene))}  # noqa: E731
    items = listed()
    assert set(items) == {"Notch", "Cope"}
    # Every section has a thumbnail of its own drawing: saved and in the model.
    assert all(i["thumb"].startswith("sketch:") for i in items.values())
    assert items["Notch"]["thumb"] != items["Cope"]["thumb"]
    key, inked, rendered = sketch_thumbnail(json.dumps(first), 96)
    assert key == items["Notch"]["thumb"] and inked > 200
    # Listed again, asked again: the same key, nothing rendered again.
    assert listed()["Notch"]["thumb"] == key
    assert sketch_thumbnail(json.dumps(first), 96) == (key, inked, rendered)
    # Renamed, saved again as it is: still the same picture.
    store(tmp_path, "sections", "Notch", rect(40, 30), rev=2)
    assert listed()["Notch"]["thumb"] == key
    # Edited and saved: another thumbnail, rendered from the new drawing.
    edited = store(tmp_path, "sections", "Notch", rect(40, 90), rev=3)
    fresh = listed()
    assert fresh["Notch"]["thumb"] != key and fresh["Cope"]["thumb"] == items["Cope"]["thumb"]
    key2, inked2, rendered2 = sketch_thumbnail(json.dumps(edited), 96)
    assert key2 == fresh["Notch"]["thumb"] and rendered2 == rendered + 1 and inked2 != inked
    # The model's section edited: its tile follows too.
    apply(h, "sketch_commit", row=sketch(rect(25, 60), uid="5ce7c400000000b1", use="section", name="Cope"))
    assert listed()["Cope"]["thumb"] != items["Cope"]["thumb"]


def test_saved_sections_and_drawn_profiles_carry_their_thumbnail(tmp_path, monkeypatch):
    from built_render.wgpu_library import ProfilesProvider, SectionsProvider

    monkeypatch.setenv("BUILT_SETTINGS_DIR", str(tmp_path))
    store(tmp_path, "sections", "Slot", rect(12, 80))
    store(tmp_path, "profiles", "Bar", rect(50, 8))
    scene = Scene()
    slot = next(i for i in SectionsProvider().items(SimpleNamespace(scene=scene)) if i["name"] == "Slot")
    assert slot["source"] == "mine" and slot["thumb"].startswith("sketch:")
    bar = next(i for i in ProfilesProvider().items(SimpleNamespace(scene=scene)) if i["name"] == "Bar")
    assert bar["thumb"].startswith("sketch:") and bar["thumb"] != slot["thumb"]
