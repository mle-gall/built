# SPDX-License-Identifier: GPL-3.0-or-later
"""Opt-in exchange timings on distinct definitions from the large-document fixture."""
import os
import resource
import sys
import time

import pytest

from built_core.scene import Scene
from formats import igz, step
from formats.skp import apply_payload
from tests.test_library_huge_perf import large_document
from tests.test_step_exchange import ruled_band

pytestmark = pytest.mark.skipif(
    os.environ.get("BUILT_STEP_SCALE") != "1", reason="opt-in exchange scale measurement"
)


def peak_mib():
    divisor = 1024**2 if sys.platform == "darwin" else 1024
    return resource.getrusage(resource.RUSAGE_SELF).ru_maxrss / divisor


@pytest.mark.parametrize("definitions", [750, 1500, 3000])
def test_faceted_exchange_scale(tmp_path, definitions):
    source = large_document(tmp_path)
    scene = Scene()
    igz.load_into(scene, source)
    # Two links, then groups of four nested grid definitions.
    roots = list(scene.groups)
    grids = definitions // 4
    tail = definitions % 4
    scene.groups = roots[:2 + grids + bool(tail)]
    if tail:
        end = scene.groups[-1]
        for _ in range(tail - 1):
            end = end.children[0]
        end.children = []
    path = tmp_path / "assembly.step"
    start = time.perf_counter()
    step.save_step(scene, path)
    exported = time.perf_counter()
    export_peak = peak_mib()
    del scene, roots
    prepared = step.prepare_step(path)
    step.apply_step(Scene(), prepared)
    imported = time.perf_counter()
    assert prepared["exact"] == definitions + 3 and not prepared["problems"]
    print(f"{definitions*676+3:,} faces, {path.stat().st_size:,} bytes, "
          f"export {exported-start:.3f} s, import {imported-exported:.3f} s, "
          f"export peak {export_peak:.0f} MiB, overall peak {peak_mib():.0f} MiB")


@pytest.mark.parametrize("definitions", [1500, 3000, 9000])
@pytest.mark.parametrize("recover", [False, True])
def test_source_arc_scale(definitions, recover):
    payload = ruled_band(n=224)
    if not recover:
        del payload["source_arcs"]
    scene = Scene()
    start = time.perf_counter()
    apply_payload(scene, {"backend": "openskp", "groups": [payload] * definitions})
    elapsed = time.perf_counter() - start
    assert len(scene.groups) == definitions
    mesh = scene.groups[-1].mesh
    assert len(mesh.faces) == 226
    assert sum(mesh._store.exact_source_strip(f.key) is not None for f in mesh.faces) == (224 if recover else 0)
    print(f"{definitions*226:,} faces, recovery {recover}, {elapsed:.3f} s, peak {peak_mib():.0f} MiB")
