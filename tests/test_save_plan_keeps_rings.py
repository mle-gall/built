# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""A lazy mesh saved twice asks once which holes are nested (#1092: the next
save, the autosave, has it), and curve and layer ids are still seen by the
quicker test: the same bytes as the mesh built and saved. Skipped without
the engine."""
from __future__ import annotations

import os
from pathlib import Path

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

import pytest  # noqa: E402

import core._native as _native  # noqa: E402
from core import mesh as M  # noqa: E402
from core import topology  # noqa: E402
from formats import igz  # noqa: E402
from tests.test_deferred_mesh import _open  # noqa: E402

pytestmark = pytest.mark.skipif(
    _native.native is None or _native.qt_hypot() is None,
    reason="engine off or not built")



def _built(mesh) -> bool:
    """A mesh built for editing: the fork's store-backed StoreMesh is one."""
    return isinstance(mesh, M.Mesh) and not isinstance(mesh, M.DeferredMesh)


def _real(face) -> bool:
    """A real face of a built mesh (the fork's FaceView too), not a light
    stand-in read from the arrays."""
    from views.array_bake import LightFace
    return isinstance(face, M.Face) and not isinstance(face, LightFace)

def _doc(curve=False, layer=False):
    return Path(__file__).parent / "fixtures" / f"nested_rings_{int(curve)}_{int(layer)}.igz"


@pytest.fixture
def binary(monkeypatch):
    monkeypatch.setattr(igz, "_LARGE_DOCUMENT", 0)
    monkeypatch.setattr(igz, "_BULK_MIN", 0)


@pytest.mark.parametrize("curve,layer", [(False, False), (True, False),
                                         (False, True), (True, True)])
def test_the_bytes_are_those_of_the_built_mesh(binary, monkeypatch, tmp_path,
                                               curve, layer):
    path = _doc(curve, layer)
    lazy = _open(monkeypatch, path, True)
    eager = _open(monkeypatch, path, False)
    assert type(lazy.mesh) is M.DeferredMesh and _built(eager.mesh)
    a, b, c = (tmp_path / n for n in ("a.igz", "b.igz", "c.igz"))
    igz.save_scene(lazy, a)
    igz.save_scene(lazy, b)
    assert type(lazy.mesh) is M.DeferredMesh, "saving built it"
    igz.save_scene(eager, c)
    assert a.read_bytes() == c.read_bytes()
    assert a.read_bytes() != path.read_bytes(), "no hole was dropped"
    assert b.read_bytes() == c.read_bytes()


def test_the_second_save_does_not_ask_again(binary, monkeypatch, tmp_path):
    path = _doc()
    lazy = _open(monkeypatch, path, True)
    calls = []
    real = topology._maximal_holes
    monkeypatch.setattr(topology, "_maximal_holes",
                        lambda h: calls.append(1) or real(h))
    igz.save_scene(lazy, tmp_path / "a.igz")
    assert len(calls) == 1
    igz.save_scene(lazy, tmp_path / "b.igz")
    assert len(calls) == 1
