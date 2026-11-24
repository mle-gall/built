# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""A store step that refuses during a push refuses the push: the model is
left as it was, the push reads as refused (no shorter push is tried), and a
repro of the mesh before and the push's arguments is written to the log
folder (docs/rust-tools.md §22 decision 1, §24)."""
from __future__ import annotations

import json

import pytest

import core.store_mesh as S
from core.protocol.commands import push_pull
from core.protocol.keys import to_json_key
from tests.test_pushpull_cases import _boxed, _face

pytestmark = pytest.mark.skipif(not S.enabled(), reason="store off")


def _state(mesh):
    return {k: v.tolist() for k, v in mesh._store.arrays().items()}


@pytest.fixture
def logs(monkeypatch, tmp_path):
    import core.paths
    import core.push_pull as P
    monkeypatch.setattr(core.paths, "user_log_dir", lambda: tmp_path)
    monkeypatch.setattr(P, "_REPROS", set())
    return tmp_path


def _refuse(name):
    """A store method body that refuses as the store does (StoreFallback,
    then the facade's refusal hook)."""
    def refusing(self, *args, **kwargs):
        try:
            raise S._fallback()(f"test: {name} refused")
        except S._fallback():
            S._note_refusal(name)
        return False
    return refusing


@pytest.mark.parametrize("where", ["translate", "stitch", "fixpoint"])
def test_a_store_refusal_leaves_the_model_and_writes_a_repro(monkeypatch, logs, where):
    scene, hist = _boxed(0, 0, 0, 2, 2, 2)
    mesh = scene.mesh
    assert type(mesh) is S.StoreMesh
    top = _face(mesh, (1, 1, 2), (0, 0, 1))
    before = _state(mesh)
    calls = []
    if where == "translate":
        from ingetrazo_core import MeshStore

        def refused_translate(self, *args):
            calls.append(1)
            raise S._fallback()("test: translation refused")

        monkeypatch.setattr(MeshStore, "push_translate", refused_translate)
    elif where == "stitch":
        refusing = _refuse("stitch_connect")

        def counted(self, *a, **k):
            calls.append(1)
            return refusing(self, *a, **k)
        monkeypatch.setattr(S.StoreMesh, "stitch_connect", counted)
    else:
        import core.cap_rebuild as C

        def refusing(mesh, *a, **k):
            calls.append(1)
            return _refuse("rebuild_push_planes")(None)
        monkeypatch.setattr(C, "rebuild_push_planes_native", refusing)
    flashes = []
    from core.protocol import commands
    monkeypatch.setattr(commands._Headless, "flash_status",
                        lambda self, text, *a: flashes.append(text), raising=False)
    push_pull(hist, to_json_key(top.key), 0.5, keep_base=where != "translate")
    assert type(mesh) is S.StoreMesh            # never handed to Python objects
    assert _state(mesh) == before
    assert len(calls) == 1                      # no shorter push tried
    repros = list((logs / "push-refusals").glob("*.json"))
    assert len(repros) == 1
    data = json.loads(repros[0].read_text())
    assert data["push"]["distance"] == pytest.approx(0.5)
    assert data["push"]["face"] == list(mesh._store.keys("face")).index(top.key)
    assert data["mesh"] == before
    assert "refused" in data["reason"]
    assert any("geometry kernel" in f for f in flashes), flashes


def test_without_a_refusal_nothing_is_written(logs):
    scene, hist = _boxed(0, 0, 0, 2, 2, 2)
    top = _face(scene.mesh, (1, 1, 2), (0, 0, 1))
    push_pull(hist, to_json_key(top.key), 0.5)
    assert not (logs / "push-refusals").exists()
