# SPDX-License-Identifier: GPL-3.0-or-later
"""Junction lane J6 (docs/junctions.md §10, §12): starter detail packs,
pack format 2 (identity, provenance, licence, units, dependencies, content
hashes), import without the source model, Share pack."""
import json
import shutil
import zipfile

import pytest

from built_core import command_protocol as live
from built_core.history import History
from built_core.protocol import commands
from built_core.scene import Scene
from formats import igz
from tests.junction_starter_cases import STARTER_PACKS, apply_in_empty_document, starter_details, sweeps, HE, IPE
from tests.test_live_junction import knee_fixture
from tests.test_live_recipes import apply


def rows_of(scene):
    return live.live_rows(scene)


def test_starter_packs_cover_the_brief():
    names = {p["pack"]: set() for p in starter_details()}
    for p in starter_details():
        names[p["pack"]].add(p["name"])
    assert set(names) == set(STARTER_PACKS)
    assert {"End plate, 4 bolts", "Fin plate", "Base plate", "Apex haunch"} <= names["Steel connections · EN"]
    assert {"Birdsmouth", "Half-lap", "Tenon and mortise", "Joist hanger"} <= names["Timber joints"]
    assert {"Corner mitre", "Butt with screws"} <= names["Generic"]
    assert {"Door opening", "Window opening", "Wall corner · by priority", "Wall tee", "Wall on slab"} <= names["Architecture"]


@pytest.mark.parametrize("name", sorted(p["name"] for p in starter_details()))
def test_every_starter_detail_applies_in_an_empty_document(name, tmp_path):
    p = next(q for q in starter_details() if q["name"] == name)
    scene, h, uid = apply_in_empty_document(p)
    rows = rows_of(scene)
    assert all(r["state"] == "ok" for r in rows), [(r.get("key"), r["kind"], r["state"], r.get("reason")) for r in rows if r["state"] != "ok"]
    j = next(r for r in rows if r["uid"] == uid)
    assert j["detail"]["uid"] == p["uid"]
    assert len(j["contents"]) >= len(p["values"]["contents"])
    # Every owned item of the template is there, and the document reopens as it was.
    path = tmp_path / "starter.built"
    igz.save_scene(scene, path)
    back = Scene()
    igz.load_into(back, path)
    assert sorted(r["uid"] for r in rows_of(back)) == sorted(r["uid"] for r in rows)
    assert all(r["state"] == "ok" for r in rows_of(back))
    apply(h, "undo")
    assert not any(r["uid"] == uid for r in rows_of(scene))


def test_fasteners_of_each_kind_share_definitions_and_file_their_class():
    p = next(q for q in starter_details() if q["name"] == "Joist hanger")
    scene, _, uid = apply_in_empty_document(p)
    parts = [g for g in scene.groups_by_uid().values() if (g.ifc or {}).get("class") == "IfcMechanicalFastener"]
    assert len(parts) == 4 and {g.ifc["predefined_type"] for g in parts} == {"NAIL"}
    assert len({id(g.mesh) for g in parts}) == 1
    # Nails make their own way: no holes in the members.
    assert not any("hole" in r.get("cutter", {}) for r in rows_of(scene))
    base = next(q for q in starter_details() if q["name"] == "Base plate")
    scene, _, _ = apply_in_empty_document(base)
    anchors = [g for g in scene.groups_by_uid().values() if (g.ifc or {}).get("predefined_type") == "ANCHORBOLT"]
    assert anchors and all(g.ifc["d"] == 16 for g in anchors)
    assert len([r for r in rows_of(scene) if "hole" in r.get("cutter", {})]) == 2


def test_block_cuts_follow_their_frame():
    from tests.test_live_cut import solid_volume
    p = next(q for q in starter_details() if q["name"] == "Half-lap")
    scene, h, uid = apply_in_empty_document(p)
    a, b = scene.groups[:2]
    stock = .045 * .145 * 2
    # Each member loses half its depth over the other's width.
    lap = .045 * .045 * .145 / 2
    assert solid_volume(a) == pytest.approx(stock - lap, rel=1e-6)
    assert solid_volume(b) == pytest.approx(stock - lap, rel=1e-6)
    apply(h, "move_placements", placements=[b.uid], delta=[.3, 0, 0])
    assert solid_volume(a) == pytest.approx(stock - lap, rel=1e-6)
    assert all(r["state"] == "ok" for r in rows_of(scene))


def test_rule_in_words_and_what_a_detail_adds():
    p = next(q for q in starter_details() if q["name"] == "End plate, 4 bolts")
    words = live.junction_words(p)
    assert words["rule"].startswith("I 100–400 beam end → I column flange")
    assert words["adds"] == "cut, part, 4 bolts, weld"
    assert words["thumb"] == "junction:end_face:bcpw"
    assert len(live.junction_thumbnail(words["thumb"], 96)) == 96 * 96 * 4


def export_knee(tmp_path, name="knee.builtpack", **meta):
    """A pack written from the garage knee: the detail and what it needs."""
    scene, history, uid = knee_fixture(portal=True)
    detail = next(r for r in rows_of(scene) if r["uid"] == uid)["detail"]["uid"]
    root = next(p for p, _ in live.preset_catalog(scene) if p["uid"] == detail)
    dest = tmp_path / name
    meta = {"name": "Garage knees", "version": "1.0.0", "author": "Workshop", "licence": "CC-BY-4.0", **meta}
    report = live.pack_export([root], meta, str(dest), None, scene)
    return scene, root, dest, report


def import_plan(path, known=None):
    return live.pack_read(str(path), known)


def test_pack_from_the_garage_imports_without_its_model_and_applies_to_a_new_frame(tmp_path):
    source, root, dest, report = export_knee(tmp_path)
    assert report["rows"] == 3          # the knee, its bolt and its weld
    with zipfile.ZipFile(dest) as z:
        manifest = json.loads(z.read("pack.json"))
        names = set(z.namelist())
    pack = manifest["pack"]
    assert manifest["v"] == 2 and len(pack["id"]) == 32 and pack["licence"] == "CC-BY-4.0"
    assert pack["units"] == {"length": "mm", "angle": "deg"} and "junction.v1" in pack["requires"]["features"]
    assert set(pack["files"]) | {"pack.json"} == names
    plan = import_plan(dest)
    by_name = {r["name"]: r for r in plan["rows"]}
    knee = by_name[root["name"]]
    assert "needs" not in knee and knee["source"]["pack"] == pack["id"] and knee["author"]["name"] == "Workshop"
    assert {(d["kind"], d["bundled"] is None) for d in knee["depends"]} == {("preset", True), ("detail", False)}
    # No document id, path or user name travels.
    assert "provenance" in knee and str(tmp_path) not in json.dumps(plan["rows"])

    # A new document, never seen the source: a new frame, the pack's rows only.
    scene = Scene()
    h = History(scene)
    column, beam = sweeps(h, (HE, [[5, 0, 0], [5, 0, 1.1]]), (IPE, [[5, 0, .8], [5, 1, 1.068]]))
    assert not any(p["uid"] == knee["uid"] for p, s in live.preset_catalog(scene) if s == "model")
    bindings = live.junction_bind(scene, knee, [beam, column])     # picked in either order
    assert bindings["main"]["placement"] == column and bindings["joining"]["end"] == "start"
    defs = live.junction_definitions(knee, plan["rows"])
    uid = apply(h, "junction_apply", detail=knee["uid"], bindings=bindings, definitions=defs)["uid"]
    rows = rows_of(scene)
    assert all(r["state"] == "ok" for r in rows), [(r.get("key"), r["state"], r.get("reason")) for r in rows if r["state"] != "ok"]
    src_rows = rows_of(source)
    for kind in ("sweep", "cut", "repeat", "junction"):
        assert len([r for r in rows if r["kind"] == kind]) == len([r for r in src_rows if r["kind"] == kind]), kind
    embedded = {p["uid"]: p for p, s in live.preset_catalog(scene) if s == "model"}
    assert embedded[knee["uid"]]["source"]["hash"] == knee["hash"]
    # One step: undo takes the junction and the embedded details away together.
    apply(h, "undo")
    assert not any(r["kind"] == "junction" for r in rows_of(scene))
    assert knee["uid"] not in {p["uid"] for p, s in live.preset_catalog(scene) if s == "model"}


def test_folder_packs_tampering_and_missing_dependencies(tmp_path):
    _, root, _, _ = export_knee(tmp_path, name="folder")
    folder = tmp_path / "folder"
    assert (folder / "pack.json").is_file()
    assert any(p.name.endswith(".png") for p in (folder / "thumbnails").iterdir())
    assert len(import_plan(folder)["rows"]) == 3
    # A changed file is refused by its hash, with nothing read from it.
    tampered = tmp_path / "tampered"
    shutil.copytree(folder, tampered)
    detail = next(f for f in (tampered / "details").glob("*.builtpreset") if '"plate_width"' in f.read_text())
    detail.write_text(detail.read_text().replace('joining.width + 150', 'joining.width + 250'))
    with pytest.raises(ValueError, match="hash"):
        import_plan(tampered)
    # A row whose own hash no longer matches is refused even when pack.json is re-hashed.
    manifest = json.loads((tampered / "pack.json").read_text())
    from hashlib import sha256
    rel = str(detail.relative_to(tampered))
    manifest["pack"]["files"][rel] = "sha256:" + sha256(detail.read_bytes()).hexdigest()
    (tampered / "pack.json").write_text(json.dumps(manifest))
    with pytest.raises(ValueError, match="hash"):
        import_plan(tampered)
    # A feature this Built does not have refuses the pack whole.
    future = tmp_path / "future"
    shutil.copytree(folder, future)
    manifest = json.loads((future / "pack.json").read_text())
    manifest["pack"]["requires"]["features"].append("lattice.v9")
    (future / "pack.json").write_text(json.dumps(manifest))
    with pytest.raises(ValueError, match="newer Built"):
        import_plan(future)
    # A pack without the weld detail: the knee needs it and says so; the bolt stays usable.
    partial = tmp_path / "partial"
    shutil.copytree(folder, partial)
    manifest = json.loads((partial / "pack.json").read_text())
    weld = next(r for r in manifest["rows"] if "weld" in r)
    manifest["rows"].remove(weld)
    del manifest["pack"]["files"][weld]
    (partial / weld).unlink()
    (partial / "pack.json").write_text(json.dumps(manifest))
    plan = import_plan(partial)
    by_name = {r["name"]: r for r in plan["rows"]}
    assert by_name[root["name"]]["needs"] and "detail" in by_name[root["name"]]["needs"][0]
    assert "needs" not in by_name["Bolt M16"]
    with pytest.raises(ValueError, match="needs"):
        live.junction_definitions(by_name[root["name"]], plan["rows"])
    # Resolved once the Library holds the missing detail.
    missing = by_name[root["name"]]["needs"][0].split()[-1]
    again = import_plan(partial, {missing: ""})
    assert all("needs" not in r for r in again["rows"])


def test_template_validation_refuses_foreign_references(tmp_path):
    p = dict(next(q for q in starter_details() if q["name"] == "Corner mitre"))
    p = json.loads(json.dumps(p))
    p["uid"] = "c0d0000000000001"
    p["values"]["params"][0]["expr"] = "brace.width / 2"
    with pytest.raises(ValueError, match="brace"):
        live.pack_export([p], {"name": "Bad"}, str(tmp_path / "bad"))
    assert not (tmp_path / "bad").exists()
    p["values"]["params"][0]["expr"] = "0"
    folder = tmp_path / "good"
    live.pack_export([p], {"name": "Good"}, str(folder))
    # A hand-edited row that names a foreign role is refused on reading.
    manifest = json.loads((folder / "pack.json").read_text())
    path = manifest["rows"][0]
    row = json.loads((folder / path).read_text())
    row["values"]["params"][0]["expr"] = "brace.width"
    row["hash"] = live.pack_row_hash(row)
    (folder / path).write_text(json.dumps(row))
    from hashlib import sha256
    manifest["pack"]["files"][path] = "sha256:" + sha256((folder / path).read_bytes()).hexdigest()
    (folder / "pack.json").write_text(json.dumps(manifest))
    with pytest.raises(ValueError, match="brace"):
        import_plan(folder)


def test_format_one_files_stay_readable(tmp_path):
    v1 = tmp_path / "old.json"
    p = next(q for q in starter_details() if q["name"] == "Corner mitre")
    v1.write_text(json.dumps({"format": "built.presets", "v": 1, "rows": [p]}))
    plan = live.pack_read(str(v1))
    assert plan["v"] == 1 and plan["rows"][0]["uid"] == p["uid"] and "needs" not in plan["rows"][0]


def test_apply_refuses_a_wrong_selection_with_the_roles_it_needs():
    p = next(q for q in starter_details() if q["name"] == "Fin plate")
    scene = Scene()
    h = History(scene)
    one, = sweeps(h, (HE, [[0, 0, 0], [0, 0, 3]]))
    with pytest.raises(ValueError, match="Select 2 members"):
        live.junction_bind(scene, p["uid"], [one])
    reply = commands.run(h, "junction_apply", detail=p["uid"], bindings={"main": {"placement": one}})
    assert not reply["ok"]


def test_fin_plate_bolts_drill_both_plies():
    fin = next(p for p in starter_details() if p["name"] == "Fin plate")
    scene, _, uid = apply_in_empty_document(fin)
    rows = rows_of(scene)
    parent = next(r for r in rows if r["uid"] == uid)
    bolts = next(r for r in rows if r.get("key") == "bolts" and r["parent"] == uid)
    grid = next(r for r in rows if r.get("key") == "bolts:repeat")
    assert grid["rule"]["grid"]["pitch"] == parent["params"]["fin_h"] / 2
    assert {r["owner"] for r in rows if r.get("junction") == bolts["uid"] and r["kind"] == "cut"} == {
        bolts["roles"][ply]["placement"] for ply in ("ply_1", "ply_2")
    }


def test_imperial_steel_details_follow_units():
    scene = Scene()
    scene.units = {"length": "mm"}
    name = "Steel connections · AISC"
    assert all(p["pack"] != name for p, _ in live.preset_catalog(scene))
    scene.units = {"length": "ft-in-frac"}
    rows = [p for p, _ in live.preset_catalog(scene) if p["pack"] == name]
    assert len(rows) == 8
    h = History(scene)
    w = {"library": "aisc/w8x18"}
    picks = sweeps(h, (w, [[0, 0, 0], [0, 0, 3]]), (w, [[0, 0, 2], [0, 2, 2]]))
    detail = next(p for p in rows if p["name"] == "Fin plate")
    apply(h, "junction_apply", detail=detail["uid"], bindings=live.junction_bind(scene, detail, picks))
    assert all(r["state"] == "ok" for r in rows_of(scene))
    bolts = [g for g in scene.groups_by_uid().values() if (g.ifc or {}).get("d") == 19.05]
    assert bolts and all("3/4" in g.name for g in bolts)


def test_component_bundle_embeds_once_on_apply(tmp_path):
    from tests.junction_starter_cases import box
    source = Scene()
    component = box(source, "Cleat", "IfcDiscreteAccessory", (0, 0, 0), (.06, .04, .008))
    component.component = True
    root = next(p for p in starter_details() if p["name"] == "Corner mitre")
    root = dict(root, uid="fed0000000000001", name="Corner with cleat")
    root["values"] = dict(root["values"], contents=[
        {"key": "cleat", "part": {"component": component.uid, "at": {"frame": "@meet"}}}
    ])
    path = tmp_path / "cleats.builtpack"
    live.pack_export([root], {"name": "Cleats"}, str(path), None, source)
    imported, = live.pack_read(str(path))["rows"]
    scene = Scene()
    h = History(scene)
    for y in (0, 2):
        picks = sweeps(h, (HE, [[0, y, 0], [2, y, 0]]), (HE, [[2, y, 0], [2, y + 1, 0]]))
        uid = apply(h, "junction_apply", detail=imported["uid"], bindings=live.junction_bind(scene, imported, picks),
                    definitions=[imported])["uid"]
        kept, = [g for g in scene.kept_components if g.uid == component.uid]
        row = next(r for r in rows_of(scene) if r["uid"] == uid)
        assert row["state"] == "ok"
        part = scene.groups_by_uid()[row["contents"][0]["placement"]]
        assert part.mesh is kept.mesh


@pytest.mark.parametrize("filename", [
    "steel-connections-en", "steel-connections-aisc", "purlin-connections-en", "mep-penetrations",
])
def test_inline_pack_hashes_and_dependencies(filename):
    from pathlib import Path
    path = Path(__file__).resolve().parents[1] / "resources" / "presets" / "packs" / (filename + ".builtpreset")
    pack = live.pack_read(str(path))
    assert pack["v"] == 2
    assert all("needs" not in row and row["source"]["hash"] == row["hash"] for row in pack["rows"])


def test_purlin_seats_at_an_oblique_crossing():
    scene = Scene()
    h = History(scene)
    a, b = sweeps(h, (HE, [[-1.5, 0, 0], [1.5, 0, 0]]), (IPE, [[-.4, -1, 0], [.4, 1, 0]]))
    p = next(p for p in starter_details() if p["name"] == "Purlin cleat")
    suggestion, = live.junction_suggest(scene, p["uid"])["suggestions"]
    uid = apply(h, "junction_apply", detail=p["uid"], bindings=suggestion["roles"])["uid"]
    rows = rows_of(scene)
    assert all(r["state"] == "ok" for r in rows)
    joint = next(r for r in rows if r["uid"] == uid)
    assert joint["frames"]["main.seat"][10] == pytest.approx(1)
    assert joint["frames"]["joining.bottom"][14] == pytest.approx(joint["frames"]["main.seat"][14])
    apply(h, "undo")
    joining = next(r for r in rows_of(scene) if r["owner"] == b)
    assert all(pt[2] == 0 for pt in joining["path"]["points"])


@pytest.mark.parametrize("name,thickness", [
    ("Pipe through wall", .2), ("Pipe through slab", .25),
    ("Duct through wall", .2), ("Duct through slab", .25),
])
def test_penetration_removes_the_clear_opening(name, thickness):
    from math import sin, tau
    from tests.test_live_cut import solid_volume

    p = next(q for q in starter_details() if q["name"] == name)
    scene, h, uid = apply_in_empty_document(p)
    j = next(r for r in rows_of(scene) if r["uid"] == uid)
    host = j["roles"]["main"]["placement"]
    cut = solid_volume(scene.groups_by_uid()[host])
    width = j["params"]["opening_w"] / 1000
    height = j["params"]["opening_h"] / 1000
    # The bore has 24 sides, matching the sleeve's faceted section.
    area = 12 * sin(tau / 24) * (width / 2) ** 2 if name.startswith("Pipe") else width * height
    apply(h, "undo")
    assert solid_volume(scene.groups_by_uid()[host]) - cut == pytest.approx(area * thickness)
