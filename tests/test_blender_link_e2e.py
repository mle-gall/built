# SPDX-License-Identifier: GPL-3.0-or-later
"""The Blender link with a real Blender (opt-in: BUILT_BLENDER_TESTS=1 and
a Blender on PATH or found by Built). Built's drawer loop runs here; the
add-on runs in a background Blender driven by blender_link_driver.py."""
from __future__ import annotations

import json
import os
import shutil
import subprocess
import time
from pathlib import Path

import numpy as np
import pytest
from tests.shell_isolation import restore_shell_patches  # noqa: F401

from built_core import render_blender as rb
from tests.test_blender_link import PLUGIN, _drawer, _matrix, _scene

ROOT = Path(__file__).resolve().parents[1]


def _blender():
    exe = shutil.which("blender")
    if exe:
        return [exe]
    found = rb.find_blender() if hasattr(rb, "find_blender") else None
    return list(found.command) if found else None


pytestmark = [pytest.mark.slow,
              pytest.mark.skipif(os.environ.get("BUILT_BLENDER_TESTS") != "1",
                                 reason="set BUILT_BLENDER_TESTS=1 to run Blender")]


def test_a_real_blender_receives_the_model_and_sends_changes_back(tmp_path, monkeypatch):
    command = _blender()
    if not command:
        pytest.skip("no Blender on this machine")
    monkeypatch.setattr(rb, "LINK_PORT", 0)
    from built_core.history import SetPluginDataCommand
    from built_core.materials import Material
    from built_core.protocol import commands as C
    sc, h, seat, leg = _scene()
    C.run(h, "paint_placements", placements=[seat.uid], paint={"color": [0.6, 0.4, 0.2], "mat": "Oak"},
          material=Material("Oak", color=(0.6, 0.4, 0.2)))
    h.execute(SetPluginDataCommand(PLUGIN, {"lights": [{"kind": "point", "pos": [1, 1, 2.5], "power": 400,
                                                         "id": "bulb", "name": "Bulb"}]}))
    link, app = _drawer(sc, h)
    pairing = link.start()
    report = tmp_path / "report.json"
    proc = subprocess.Popen([*command, "-b", "--factory-startup", "--python", str(ROOT / "tests/blender_link_driver.py"),
                             "--", str(ROOT / "integrations/blender"), pairing, "3", str(report)],
                            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, env=rb.clean_env())
    applied = False
    end = time.monotonic() + 180
    while proc.poll() is None and time.monotonic() < end:
        link.tick()
        if len(link.review) >= 3 and not applied:
            link.event("link_apply_all", "invoke", None)
            applied = True
        time.sleep(0.02)
    if proc.poll() is None:
        proc.kill()
    out = proc.stdout.read().decode(errors="replace")
    link.close()
    assert report.exists(), out[-3000:]
    got = json.loads(report.read_text())
    assert "error" not in got, got.get("error")
    assert got["linked"] and got["model"], out[-3000:]
    assert set(got["roots"]) == {seat.uid, leg.uid, "loose"}
    assert got["roots"][seat.uid]["matrix"][0][3] == pytest.approx(2.0, abs=1e-5)   # Built's matrix
    assert got["materials"]["Oak"]["color"] == pytest.approx([0.6, 0.4, 0.2], abs=1e-4)
    assert got["lights"] == ["bulb"]
    assert got["document"] == app.scene.plugin_data[PLUGIN]["link_id"]
    assert got["changes_before_edit"] == 0, "Built's own values read back as no change"
    assert got["sent"] == 3 and got["answered"] and applied
    assert got["changes_after"] == []
    assert _matrix(seat)[0, 3] == pytest.approx(2.5, abs=1e-4)
    assert sc.materials.get("Oak").pbr["roughness"] == pytest.approx(0.3, abs=1e-4)
    names = [lt["name"] for lt in sc.plugin_data[PLUGIN]["lights"]]
    assert names == ["Bulb", "Studio key"]
    assert np.allclose(_matrix(leg), np.eye(4))


def test_a_new_textured_material_from_blender_comes_back_and_renders(tmp_path, monkeypatch):
    command = _blender()
    if not command:
        pytest.skip("no Blender on this machine")
    monkeypatch.setattr(rb, "LINK_PORT", 0)
    sc, h, seat, leg = _scene()
    link, app = _drawer(sc, h)
    pairing = link.start()
    report = tmp_path / "report.json"
    env = {**rb.clean_env(), "DRIVER_MODE": "newmaterial"}
    proc = subprocess.Popen([*command, "-b", "--factory-startup", "--python", str(ROOT / "tests/blender_link_driver.py"),
                             "--", str(ROOT / "integrations/blender"), pairing, "3", str(report)],
                            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, env=env)
    applied = False
    end = time.monotonic() + 180
    while proc.poll() is None and time.monotonic() < end:
        link.tick()
        if len(link.review) >= 1 and not applied:
            rows = link.rows()
            assert rows[0]["cells"][0] == "New material: Checker", rows
            link.event("link_apply_all", "invoke", None)
            applied = True
        time.sleep(0.02)
    if proc.poll() is None:
        proc.kill()
    out = proc.stdout.read().decode(errors="replace")
    link.close()
    got = json.loads(report.read_text())
    assert "error" not in got, got.get("error")
    assert got["new_changes"] == ["newmaterial:Checker"], out[-2000:]
    assert got["sent"] == 1 and got["answered"] and applied
    assert got["adopted"] == "Checker" and got["changes_after"] == []
    m = sc.materials.get("Checker")
    assert m is not None and Path(m.texture["path"]).is_file()
    assert m.pbr["roughness"] == pytest.approx(0.3, abs=1e-4)
    assert all(f.attrs.get("mat") == "Checker" for f in leg.mesh.faces)
    # Built draws and exports it: its image goes out as the base colour.
    from formats.gltf import save_glb
    from formats.glb import parse_container
    save_glb(sc, tmp_path / "check.glb")
    js, _ = parse_container(tmp_path / "check.glb")
    (mat,) = [m for m in js["materials"] if m["name"] == "Checker"]
    assert "baseColorTexture" in mat["pbrMetallicRoughness"] and js["images"]
