# SPDX-License-Identifier: GPL-3.0-or-later
"""Junction details in Library › Presets (docs/junctions.md §10, §11): the
Junction filter, the rule in words, Apply… on the selection, packs imported
into a Library destination with their dependencies checked, update offers.
Everything stays inside the test profile."""
import json
import shutil

import pytest
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from built_core import command_protocol as live

from built_render import wgpu_library as L
from tests.junction_starter_cases import HE, IPE, sweeps
from tests.test_junction_packs import export_knee
from tests.test_library_destinations import Renderer, act, frame, tiles, win  # noqa: F401


def presets_sheet(win, facet="junction", scope="s:bundled"):
    L.open_library(win, "presets")
    frame(win)
    act(win, "side", scope)
    act(win, "headmenu", "show", f"facet:{facet}")
    return frame(win)


def detail_rows(sheet):
    return {r[0]: r[1] for b in sheet["detail"]["blocks"] if b["kind"] == "kv" for r in b["rows"]}


def buttons(sheet):
    return [x["label"] for b in sheet["detail"]["blocks"] if b["kind"] == "buttons" for x in b["buttons"]]


def test_junction_filter_rule_in_words_and_apply_on_the_selection(win):
    sheet = presets_sheet(win)
    shown = {i["label"]: i for i in tiles(sheet)}
    assert {"End plate, 4 bolts", "Half-lap", "Corner mitre", "Wall tee"} <= set(shown)
    assert "Plate" not in shown and "Mitre" not in shown          # sweeps and cuts filtered out
    end_plate = shown["End plate, 4 bolts"]
    assert end_plate["sub"].startswith("I 100–400 beam end → I column flange")
    act(win, "focus", end_plate["id"])
    sheet = frame(win)
    rows = detail_rows(sheet)
    assert rows["Adds"] == "cut, part, 4 bolts, weld" and rows["Pack"].startswith("Steel connections · EN")
    assert rows["Licence"] == "GPL-3.0-or-later" and rows["Plate thickness"] == "15 mm"
    assert "Apply…" in buttons(sheet) and "Share pack…" in buttons(sheet)
    # Apply… binds the selection: two members, either order.
    sc = win.viewport.scene
    h = win.viewport.history
    column, beam = sweeps(h, (HE, [[0, 0, 0], [0, 0, 3]]), (IPE, [[0, 0, 2], [0, 2, 2]]))
    by = sc.groups_by_uid()
    sc.select([by[beam], by[column]])
    act(win, "menu", end_plate["id"], "apply")
    rows = live.live_rows(sc)
    j = next(r for r in rows if r["kind"] == "junction" and not r.get("parent"))
    assert j["detail"]["name"] == "End plate, 4 bolts" and all(r["state"] == "ok" for r in rows)
    h.undo()
    assert not any(r["kind"] == "junction" for r in live.live_rows(sc))
    # The presets facet never hides materials.
    from tests.test_wgpu_library import _items, _state
    items = _items()
    items[0]["base"] = "colour"
    assert len(L.filtered(items, _state(facet="junction"))) == len(items)
    items[1]["base"] = "sweep"
    assert [e["key"] for e in L.filtered(items, _state(facet="junction"))] != [e["key"] for e in items]


def test_import_into_my_library_needs_and_tampering(win, tmp_path):
    _, root, pack, _ = export_knee(tmp_path, name="folder")
    # A pack missing the weld detail: the knee needs it. This model takes only
    # what is complete; My library lists the knee with what it needs.
    partial = tmp_path / "partial"
    shutil.copytree(pack, partial)
    manifest = json.loads((partial / "pack.json").read_text())
    weld = next(r for r in manifest["rows"] if "weld" in r)
    manifest["rows"].remove(weld)
    del manifest["pack"]["files"][weld]
    (partial / weld).unlink()
    (partial / "pack.json").write_text(json.dumps(manifest))
    assert L.import_pack(win, str(partial), "model") == {"ok": 1, "needs": 0, "skipped": 1, "destination": "This model"}
    assert L.import_pack(win, str(partial), "mine")["needs"] == 1
    sheet = presets_sheet(win, scope="s:mine")
    knee = next(i for i in tiles(sheet) if i["label"] == root["name"])
    act(win, "focus", knee["id"])
    sheet = frame(win)
    assert detail_rows(sheet)["Needs"].startswith("detail ") and "Apply…" not in buttons(sheet)
    # The whole pack completes it.
    report = L.import_pack(win, str(pack), "mine")
    assert report == {"ok": 3, "needs": 0, "skipped": 0, "destination": "My library"}
    sheet = presets_sheet(win, scope="s:mine")
    act(win, "focus", knee["id"])
    sheet = frame(win)
    rows = detail_rows(sheet)
    assert rows["Pack"] == "Garage knees 1.0.0" and rows["Author"] == "Workshop" and rows["Licence"] == "CC-BY-4.0"
    assert "Needs" not in rows and "Apply…" in buttons(sheet)
    assert {i["category"] for i in L.library_items("presets", win.viewport.scene) if i["source"] == "mine"} == {"Garage knees"}
    # A tampered pack is refused whole: nothing changes in My library.
    import os
    from pathlib import Path
    mine = Path(os.environ["BUILT_SETTINGS_DIR"]) / "data/Built/Library/presets"
    before = {p.name: p.read_text() for p in mine.glob("*.json")}
    tampered = tmp_path / "tampered"
    shutil.copytree(pack, tampered)
    detail = next((tampered / "details").glob("*.builtpreset"))
    detail.write_text(detail.read_text().replace('"rev": 1', '"rev": 9', 1))
    with pytest.raises(ValueError, match="hash"):
        L.import_pack(win, str(tampered), "mine")
    assert {p.name: p.read_text() for p in mine.glob("*.json")} == before


def test_newer_pack_revision_is_offered_never_applied(win, tmp_path):
    source, root, first, _ = export_knee(tmp_path, name="v1.builtpack", id="ab" * 16)
    plan = live.pack_read(str(first))
    knee = next(r for r in plan["rows"] if r["name"] == root["name"])
    sc = win.viewport.scene
    h = win.viewport.history
    column, beam = sweeps(h, (HE, [[5, 0, 0], [5, 0, 1.1]]), (IPE, [[5, 0, .8], [5, 1, 1.068]]))
    bindings = live.junction_bind(sc, knee, [column, beam])
    from built_core.protocol import commands
    assert commands.run(h, "junction_apply", detail=knee["uid"], bindings=bindings,
                        definitions=live.junction_definitions(knee, plan["rows"]))["ok"]
    # The same pack, revision 2 of the knee, lands in My library.
    root2 = dict(root, rev=2)
    root2["values"] = json.loads(json.dumps(root["values"]))
    root2["values"]["params"][0]["expr"] = "joining.width + 160"
    rows = [p for p, _ in live.preset_catalog(source)]
    second = tmp_path / "v2.builtpack"
    live.pack_export([root2], {"name": "Garage knees", "id": "ab" * 16}, str(second), rows, source)
    L.import_pack(win, str(second), "mine")
    row = next(i for i in L.library_items("presets", sc) if i["source"] == "mine" and i["name"] == root["name"])
    assert row["preset"]["primitive"] == "junction"
    sheet = presets_sheet(win, scope="s:mine")
    item = next(i for i in tiles(sheet) if i["label"] == root["name"])
    act(win, "focus", item["id"])
    sheet = frame(win)
    assert detail_rows(sheet)["Update available"] == "rev 1 → 2 · 1 use"
    model = {p["uid"]: p for p, s in live.preset_catalog(sc) if s == "model"}
    assert model[knee["uid"]]["source"]["rev"] == 1          # offered, not applied
    act(win, "menu", item["id"], "modelupdate")
    model = {p["uid"]: p for p, s in live.preset_catalog(sc) if s == "model"}
    assert model[knee["uid"]]["source"]["rev"] == 2
    j = next(r for r in live.live_rows(sc) if r["kind"] == "junction" and not r.get("parent"))
    assert j["params"]["plate_width"] == 120 + 160 and j["state"] == "ok"
