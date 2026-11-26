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
