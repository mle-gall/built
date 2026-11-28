"""The provisional steel profile packs: sources, generator and validation.

Every size in addons/steel-profiles-*/ is drawn with Built's parametric
families; its computed area and mass must match the listed values within
1 % (2 % for sloped flanges), every size cites a recorded source and is
provisional, and the committed rows are what the generator writes.
"""
import csv
import json
from pathlib import Path

import pytest

from scripts import build_steel_profiles as gen
from tests.addon_cases import rows

ROOT = Path(__file__).resolve().parents[1]
PACKS = gen.folders()


def test_every_steel_pack_has_sources_and_provisional_sizes():
    assert {f.name for f in PACKS} >= {"steel-profiles-en", "steel-profiles-aisc"}
    for folder in PACKS:
        src = json.loads((folder / "source/sources.json").read_text(encoding="utf-8"))
        assert src["status"] == "provisional"
        for so in src["sources"].values():
            assert so["url"].startswith("https://") and so["accessed"] and so["title"]
        for fam in src["families"]:
            with open(folder / "source" / fam["file"], newline="", encoding="utf-8") as f:
                sizes = list(csv.DictReader(f))
            assert sizes, fam["file"]
            assert list(sizes[0]) == ["designation", "h", "b", "tw", "tf", "r1", "r2", "slope", "mass", "area", "status", "source"]
            for s in sizes:
                assert s["status"] == "provisional"
                assert all(sid in src["sources"] for sid in gen.source_ids(s["source"])), s


@pytest.mark.parametrize("folder", PACKS, ids=lambda f: f.name)
def test_computed_area_and_mass_match_the_listed_values(folder):
    src, kept, refused, stats = gen.evaluate(folder)
    assert kept
    for e in kept:
        p = e["props"]
        tol = gen.TOLERANCE_SLOPED if e["family"].get("sloped") else gen.TOLERANCE
        s = e["size"]
        mass = p["area"] * 1e-6 * gen.DENSITY
        if e["family"].get("mass") != "nominal":
            assert abs(mass / s["mass"] - 1) <= tol, e["key"]
        if not (e["family"]["shape"] in ("rhs", "chs") and s["tf"] != s["tw"]):
            assert abs(p["area"] / 100 / s["area"] - 1) <= tol, e["key"]
    # Every size that stays out is listed with its reason.
    report = (folder / "source/report.md").read_text(encoding="utf-8")
    for e in refused:
        assert e["why"] and f"| {e['size']['designation']} |" in report


def test_committed_rows_are_what_the_generator_writes():
    for folder in PACKS:
        differs, kept, refused = gen.generate(folder, check=True)
        assert not differs, f"{folder.name}: run scripts/build_steel_profiles.py ({differs[:5]})"
        manifest = json.loads((folder / "pack.json").read_text(encoding="utf-8"))
        sizes = [json.loads((folder / n).read_text(encoding="utf-8")) for n in manifest["rows"]]
        sizes = [r for r in sizes if r.get("profile_size")]
        assert len(sizes) == len(kept)
        for r in sizes:
            assert r["provenance"]["status"] == "provisional" and r["provenance"]["source"].startswith("https://")
            assert r["values"]["profile"]["library"].startswith(json.loads((folder / "source/sources.json").read_text())["prefix"] + "/")


def test_keys_are_unique_and_existing_keys_keep_their_uid():
    for folder in PACKS:
        manifest = json.loads((folder / "pack.json").read_text(encoding="utf-8"))
        keys = [json.loads((folder / n).read_text(encoding="utf-8"))["values"]["profile"]["library"]
                for n in manifest["rows"]
                if json.loads((folder / n).read_text(encoding="utf-8")).get("profile_size")]
        assert len(keys) == len(set(keys)), folder.name
    en = {r["values"]["profile"]["library"]: r["uid"] for r in rows("steel-profiles-en") if r.get("profile_size")}
    assert en["en/ipe200"] == "b9e25a2c531a7322" and en["en/he200a"] == "e150271952831c8a"
    for key in ("en/ipe240", "en/upe200", "en/l50x5", "en/rhs100x5", "en/rhs100x4", "en/shs80x4", "en/chs114.3x5"):
        assert key in en
    aisc = {r["values"]["profile"]["library"] for r in rows("steel-profiles-aisc") if r.get("profile_size")}
    assert "aisc/w8x18" in aisc


def test_packs_import_and_every_size_draws():
    from built_core import command_protocol
    for folder in PACKS:
        imported = rows(folder.name)
        assert not [r for r in imported if r.get("needs")]
        for r in imported:
            if r.get("profile_size"):
                command_protocol.profile_properties(r["values"]["profile"], 64)


def test_sloped_flanges_and_toe_radii_change_the_drawing():
    from built_core import command_protocol
    ipn200 = {"height": 200, "width": 90, "web": 7.5, "flange": 11.3, "radius": 7.5}
    plain = command_protocol.profile_properties({"family": "i", "params": ipn200})
    sloped = command_protocol.profile_properties({"family": "i", "params": dict(ipn200, toe=4.5, slope=14, flange_at=22.5)})
    # EN 10365 lists IPN 200 at 33.4 cm²; the parallel-flange drawing without toes is 2 % heavier.
    assert abs(sloped["area"] / 3340 - 1) < 0.005
    assert plain["area"] > sloped["area"] * 1.015
    angle = {"height": 100, "width": 100, "web": 6, "radius": 12}
    # A toe radius equal to the leg thickness uses the whole leg end.
    full = command_protocol.profile_properties({"family": "angle", "params": dict(angle, toe=6)})
    assert full["area"] < command_protocol.profile_properties({"family": "angle", "params": angle})["area"]
