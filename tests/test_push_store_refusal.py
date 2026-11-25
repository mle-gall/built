# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""A store step that refuses during a push refuses the push: the model is
left as it was, the push reads as refused (no shorter push is tried), and a
repro of the mesh before and the push's arguments is written to the log
folder."""
from __future__ import annotations

import json

import pytest

import built_core.store_mesh as S
from built_core.protocol.commands import push_pull
from built_core.protocol.keys import to_json_key
from tests.test_pushpull_cases import _boxed, _face



def _state(mesh):
    return {k: v.tolist() for k, v in mesh._store.arrays().items()}


@pytest.fixture
def logs(monkeypatch, tmp_path):
    import built_core.paths
    import built_core.push_pull as P
    monkeypatch.setattr(built_core.paths, "user_log_dir", lambda: tmp_path)
    monkeypatch.setattr(P, "_REPROS", set())
    return tmp_path


@pytest.mark.parametrize("where", ["translate", "extrude"])
def test_a_store_refusal_leaves_the_model_and_writes_a_repro(monkeypatch, logs, where):
    scene, hist = _boxed(0, 0, 0, 2, 2, 2)
    mesh = scene.mesh
    assert type(mesh) is S.StoreMesh
    top = _face(mesh, (1, 1, 2), (0, 0, 1))
    before = _state(mesh)
    calls = []
    from built_core import MeshStore

    def refused(self, *args):
        calls.append(1)
        raise S.StoreFallback(f"test: {where} refused")

    monkeypatch.setattr(MeshStore, "push_" + where, refused)
    flashes = []
    from built_core.push_pull import PushPullOperation
    commit = PushPullOperation._commit

    def observe_commit(operation, viewport):
        monkeypatch.setattr(type(viewport), "flash_status",
                            lambda self, text, *args: flashes.append(text))
        return commit(operation, viewport)

    monkeypatch.setattr(PushPullOperation, "_commit", observe_commit)
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
