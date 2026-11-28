# SPDX-License-Identifier: GPL-3.0-or-later
"""Trade add-ons (addons/<pack-id>/, docs/library.md › Trade add-ons): each
source folder is a consistent format-2 pack under CC0-1.0, builds into the
same .builtpack bytes every time, refuses edited files until --rehash, and
never ships inside the app's resources."""
import json
import shutil
import zipfile
from pathlib import Path

import pytest

from scripts import build_addons

PACKS = sorted(f.name for f in build_addons.folders())


def test_every_trade_pack_is_a_source_folder_with_its_own_id():
    assert PACKS == ["mep-penetrations", "purlin-connections-en", "steel-connections-aisc",
                     "steel-connections-en", "steel-profiles-aisc", "steel-profiles-en", "timber-joints"]
    manifests = [json.loads((build_addons.ADDONS / p / "pack.json").read_text()) for p in PACKS]
    assert len({m["pack"]["id"] for m in manifests}) == len(PACKS)
    for m in manifests:
        assert m["format"] == "built.presets" and m["v"] == 2
        assert m["pack"]["licence"] == "CC0-1.0"
        assert m["pack"]["description"] and m["pack"]["version"] and m["pack"]["requires"]["built"]
        assert sorted(m["rows"]) == sorted(m["pack"]["files"])
    root = build_addons.ROOT / "resources"
    assert not any(root.rglob("*.builtpack"))
    assert not (root / "presets" / "packs" / "steel-connections-en.builtpreset").exists()


@pytest.mark.parametrize("pack", PACKS)
def test_sources_are_rehashed_and_build_the_same_bytes(pack, tmp_path):
    from built_core import command_protocol

    folder = tmp_path / "src" / pack
    shutil.copytree(build_addons.ADDONS / pack, folder)
    assert build_addons.rehash(folder) == []
    first = build_addons.build(folder, tmp_path / "a").read_bytes()
    second = build_addons.build(folder, tmp_path / "b").read_bytes()
    assert first == second
    with zipfile.ZipFile(tmp_path / "a" / f"{pack}.builtpack") as archive:
        assert {i.date_time for i in archive.infolist()} == {(1980, 1, 1, 0, 0, 0)}
    plan = command_protocol.pack_read(str(tmp_path / "a" / f"{pack}.builtpack"))
    assert plan["v"] == 2 and all(r["source"]["hash"] == r["hash"] for r in plan["rows"])
    assert all(r["provenance"]["created_with"] == "Built" for r in plan["rows"])


def test_an_edited_row_is_refused_until_rehashed_and_a_new_row_is_listed(tmp_path):
    folder = tmp_path / "steel-profiles-en"
    shutil.copytree(build_addons.ADDONS / folder.name, folder)
    path = next((folder / "rows").glob("*.builtpreset"))
    row = json.loads(path.read_text())
    row["name"] += " (edited)"
    path.write_text(json.dumps(row, indent=2, ensure_ascii=False) + "\n")
    with pytest.raises(ValueError, match="--rehash"):
        build_addons.build(folder, tmp_path / "dist")
    extra = dict(row, uid="0123456789abcdef", name="IPE 270")
    (folder / "rows" / "0123456789abcdef.builtpreset").write_text(json.dumps(extra, indent=2) + "\n")
    changed = {p.name for p in build_addons.rehash(folder)}
    assert changed == {path.name, "0123456789abcdef.builtpreset", "pack.json"}
    manifest = json.loads((folder / "pack.json").read_text())
    assert manifest["rows"][-1] == "rows/0123456789abcdef.builtpreset"
    from built_core import command_protocol
    rows = command_protocol.pack_read(str(build_addons.build(folder, tmp_path / "dist")))["rows"]
    assert {"IPE 270", row["name"]} <= {r["name"] for r in rows}
    assert build_addons.rehash(folder) == []
