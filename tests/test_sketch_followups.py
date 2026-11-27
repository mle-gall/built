# SPDX-License-Identifier: GPL-3.0-or-later
"""Sketch follow-ups (general-base §4.4, §5.3): releasing a sketch leaves each
member a frozen copy of its profile and undo restores the link; a member's
catalogue size opens as an exact drawing that the member takes on Done in one
History step; Library › Sections lists drawn sections; DWG through the
LibreDWG bridge says plainly when the converter is missing."""
import json
from pathlib import Path
from types import SimpleNamespace

import pytest

from built_core import command_protocol
from built_core.history import History
from built_core.protocol import queries
from built_core.scene import Scene
from formats import igz
from tests.test_sketch_rows import apply, extent, member, positions, rect, rows_of, sketch

FIXTURES = Path(__file__).parent / "fixtures" / "sketch"


def _two_members(scene):
    h = History(scene)
    s = sketch(rect(200, 100))
    apply(h, "sketch_commit", row=s)
    apply(h, "create_sweeps", sweeps=[member(dict(sketch=s["uid"])),
                                       member(dict(sketch=s["uid"]), ((0, 1, 0), (3, 1, 0)))])
    return h, s


def _profile_of(scene, owner):
    return command_protocol.live_sweep(scene, owner)["profile"]


def test_release_gives_each_member_a_frozen_copy_and_undo_restores_the_link():
    scene = Scene()
    h, s = _two_members(scene)
    a, b = scene.groups
    before = [positions(g) for g in scene.groups]
    apply(h, "live_release", uid=s["uid"])
    copies = rows_of(scene, "sketch")
    assert len(copies) == 2 and s["uid"] not in {c["uid"] for c in copies}
    assert all(c["frozen"] is True and c["state"] == "ok" and c["name"] == "Plate" for c in copies)
    assert all(c["curves"] == rows_of(scene, "sketch")[0]["curves"] for c in copies)
    links = {_profile_of(scene, g.uid)["sketch"] for g in (a, b)}
    assert links == {c["uid"] for c in copies}, "each member its own copy"
    assert {r["state"] for r in rows_of(scene, "sweep")} == {"ok"}, "never profile_missing"
    assert [positions(g) for g in scene.groups] == before
    # Editing one member's copy moves that member only.
    mine = _profile_of(scene, a.uid)["sketch"]
    copy = next(c for c in copies if c["uid"] == mine)
    apply(h, "sketch_commit", row=dict(copy, curves=rect(300, 100)))
    assert extent(a)[1] == pytest.approx(0.3) and extent(b)[1] == pytest.approx(0.2)
    assert next(r for r in rows_of(scene, "sketch") if r["uid"] == mine)["frozen"] is True
    # Undo the edit, then the release: one sketch again, both members on it.
    apply(h, "undo")
    apply(h, "undo")
    (row,) = rows_of(scene, "sketch")
    assert row["uid"] == s["uid"] and "frozen" not in row
    assert {_profile_of(scene, g.uid)["sketch"] for g in (a, b)} == {s["uid"]}
    apply(h, "sketch_commit", row=dict(s, curves=rect(250, 100)))
    assert extent(a)[1] == pytest.approx(0.25) and extent(b)[1] == pytest.approx(0.25)
    apply(h, "undo")
    apply(h, "redo")  # the 250 edit again
    apply(h, "undo")
    apply(h, "redo")
    assert extent(b)[1] == pytest.approx(0.25)


def test_release_freezes_a_section_cut_s_drawing_too():
    """A notch drawn as a section keeps cutting its member after the sketch is
    released: the cut gets its own frozen copy; undo restores the link."""
    from built_core.solids import solid_volume

    scene = Scene()
    h = History(scene)
    apply(h, "create_sweeps", sweeps=[member(dict(library="basic/timber"))])
    (target,) = scene.groups
    before = solid_volume(target)
    notch = sketch(rect(100, 100), uid="5ce7c400000000b1", use="section")
    for curve in notch["curves"]:
        for point in ("a", "b"):
            x, y = curve[point]
            curve[point] = [x + 1000, y - 50]
    apply(h, "sketch_commit", row=notch)
    apply(h, "live_set", row=dict(kind="cut", owner=target.uid, uid="c070000000000101",
                                  cutter={"section": notch["uid"]}, operation="subtract"))
    cut_volume = solid_volume(target)
    assert cut_volume < before
    apply(h, "live_release", uid=notch["uid"])
    (copy,) = rows_of(scene, "sketch")
    assert copy["frozen"] is True and copy["use"] == "section"
    (cut,) = rows_of(scene, "cut")
    assert cut["cutter"] == {"section": copy["uid"]} and cut["state"] == "ok"
    assert solid_volume(target) == pytest.approx(cut_volume)
    apply(h, "undo")
    (cut,) = rows_of(scene, "cut")
    assert cut["cutter"] == {"section": notch["uid"]}
    assert [r["uid"] for r in rows_of(scene, "sketch")] == [notch["uid"]]


def test_frozen_copies_travel_through_save_and_open_and_are_not_project_profiles(tmp_path):
    from built_render.wgpu_library import ProfilesProvider

    scene = Scene()
    h, s = _two_members(scene)
    apply(h, "live_release", uid=s["uid"])
    path = tmp_path / "frozen.built"
    igz.save_scene(scene, path)
    back = Scene()
    igz.load_into(back, path)
    copies = rows_of(back, "sketch")
    assert len(copies) == 2 and all(c["frozen"] is True for c in copies)
    assert {_profile_of(back, g.uid)["sketch"] for g in back.groups} == {c["uid"] for c in copies}
    assert {r["state"] for r in rows_of(back, "sweep")} == {"ok"}
    # The Sweep picker and Library › Profiles offer project sketches, not a
    # member's own frozen copy.
    choices = ProfilesProvider().items(SimpleNamespace(scene=back))
    assert not [c for c in choices if c["source"] == "project"]


def test_a_catalogue_member_opens_as_a_drawing_and_done_adopts_it_in_one_step():
    scene = Scene()
    h = History(scene)
    apply(h, "create_sweeps", sweeps=[member(dict(library="en/ipe240"))])
    (g,) = scene.groups
    (row,) = rows_of(scene, "sweep")
    size = extent(g)
    drawn = command_protocol.profile_sketch("en/ipe240")
    assert drawn["name"] == "IPE 240" and drawn["use"] == "profile"
    kinds = sorted({c["kind"] for c in drawn["curves"]})
    assert kinds == ["arc", "line"], "root fillets stay exact arcs"
    assert len([c for c in drawn["curves"] if c["kind"] == "arc"]) == 4
    steps = len(h.undo_stack)
    reply = apply(h, "sketch_commit", row=dict(drawn, uid="5ce7c40000000009"), adopt=[row["uid"]])
    assert len(h.undo_stack) == steps + 1
    assert _profile_of(scene, g.uid) == {"sketch": reply["uid"], "params": {}}
    assert rows_of(scene, "sweep")[0]["state"] == "ok"
    assert extent(g) == pytest.approx(size)
    apply(h, "undo")
    assert rows_of(scene, "sketch") == []
    assert _profile_of(scene, g.uid)["library"] == "en/ipe240"
    assert command_protocol.profile_sketch("no/such") is None
    with pytest.raises(Exception):
        apply(h, "sketch_commit", row=dict(drawn, uid="5ce7c4000000000a"), adopt=["0000000000000bad"])


def test_library_sections_list_drawn_sections_only(tmp_path, monkeypatch):
    from built_render import sketch_workspace
    from built_render.wgpu_library import PROVIDERS, ProfilesProvider, SectionsProvider

    monkeypatch.setenv("BUILT_SETTINGS_DIR", str(tmp_path))
    body = {"use": "section", "name": "Notch", "curves": rect(40, 30),
            "extrude": {"extent": "through"}}
    sketch_workspace.save_profile(json.dumps({"format": "built.profile", "v": 1, "name": "Notch",
                                              "use": "section", "sketch": body}))
    sketch_workspace.save_profile(json.dumps({"format": "built.profile", "v": 1, "name": "Plate",
                                              "use": "profile", "sketch": dict(body, use="profile")}))
    scene = Scene()
    h = History(scene)
    apply(h, "sketch_commit", row=sketch(rect(10, 10), uid="5ce7c400000000a1", use="section", name="Cope"))
    apply(h, "sketch_commit", row=sketch(rect(10, 10), uid="5ce7c400000000a2"))
    assert [p.id for p in PROVIDERS].index("sections") == [p.id for p in PROVIDERS].index("profiles") + 1
    items = SectionsProvider().items(SimpleNamespace(scene=scene))
    assert sorted((i["name"], i["source"]) for i in items) == [("Cope", "model"), ("Notch", "mine")]
    notch = next(i for i in items if i["name"] == "Notch")
    # A saved section opens as its drawing (the Cut tool takes presets).
    assert notch["verbs"][0] == "edit" and "apply" not in notch["verbs"]
    assert ["Size", "40 × 30 mm", ""] in [list(r) for r in notch["defaults"]]
    assert ProfilesProvider().items(SimpleNamespace(scene=scene)) and not [
        i for i in ProfilesProvider().items(SimpleNamespace(scene=scene)) if i["name"] in ("Notch", "Cope")]


class _Window:
    """What sketch_workspace.enter reads before it needs a renderer."""

    def __init__(self):
        self.viewport = SimpleNamespace(scene=Scene())
        self.notices = []

    def notify(self, tone, text, actions=None):
        self.notices.append((tone, text))


def test_dwg_without_the_converter_says_so_and_never_raises(monkeypatch):
    import built_core.formats.dwg_bridge as bridge
    from built_render import sketch_workspace

    monkeypatch.setattr(bridge, "have_dwg_support", lambda: False)
    win = _Window()
    assert sketch_workspace.enter(win, "import", str(FIXTURES / "plate-mm.dwg")) is False
    ((tone, text),) = win.notices
    assert tone == "info" and "LibreDWG" in text and "DXF" in text


def test_dwg_the_converter_refuses_is_an_error_message(monkeypatch, tmp_path):
    import built_core
    import built_core.formats.dwg_bridge as bridge
    from built_render import sketch_workspace

    monkeypatch.setattr(bridge, "have_dwg_support", lambda: True)

    def refuse(path):
        raise bridge.DwgBridgeError("conversion produced no output: bad header", "failed")

    monkeypatch.setattr(built_core, "dwg_convert", refuse)
    win = _Window()
    assert sketch_workspace.enter(win, "import", str(tmp_path / "broken.dwg")) is False
    ((tone, text),) = win.notices
    assert tone == "error" and text.startswith("Could not read the DWG file") and "bad header" in text


def test_the_dwg_fixture_converts_to_the_dxf_drawing():
    """End to end through the bridge when LibreDWG is installed: the DWG
    fixture (written from plate-mm.dxf by LibreDWG's dxf2dwg, see
    tests/fixtures/sketch/SOURCES.md) reads back as the same plate."""
    import built_core
    import built_core.formats.dwg_bridge as bridge

    if not bridge.have_dwg_support():
        pytest.skip("LibreDWG's dwg2dxf is not installed")
    out = built_core.dwg_convert(str(FIXTURES / "plate-mm.dwg"))
    try:
        text = Path(str(out)).read_text(errors="replace")
    finally:
        bridge.discard_temp_dxf(out)
    def kinds(dxf):
        rows = [line.strip() for line in dxf.splitlines()]
        return sorted(r for r in rows if r in ("LINE", "ARC", "CIRCLE", "LWPOLYLINE"))

    assert kinds(text) == kinds((FIXTURES / "plate-mm.dxf").read_text())
    assert kinds(text) == ["ARC"] * 2 + ["LINE"] * 4 + ["LWPOLYLINE"] * 2
