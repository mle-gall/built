# SPDX-License-Identifier: GPL-3.0-or-later
"""Ride-scale cost of persistent ids (docs/persistent-ids.md §3.6 gates).

A synthetic document of many grid definitions, ≈ 2.4 M elements (the
largest real model has ≈ 18 M; the gates are ratios, so the scale only has
to make the costs measurable). Compared with the same file stripped of its
ids (what a format-1 file is): file size, open time, building every store,
and the resident cost of the id columns, implicit and dense."""
from __future__ import annotations

import json
import time
import zipfile

import pytest

pytestmark = pytest.mark.perf

GRID = 120          # 120 x 120 quads: 14,400 faces, 29,040 edges, 14,641 vertices
DEFINITIONS = 40    # ≈ 2.33 M elements in all


def _document(folder):
    from built_core import Matrix4, Vec3 as V
    from built_core.group import Group
    from built_core.scene import Scene
    from formats import igz

    seed = Scene()
    grid = Group(name="Grid")
    grid.xform = Matrix4()
    for y in range(GRID):
        for x in range(GRID):
            grid.mesh.add_face([V(x, y, 0), V(x + 1, y, 0), V(x + 1, y + 1, 0), V(x, y + 1, 0)])
    seed.groups.append(grid)
    path = folder / "seed.built"
    igz.save_scene(seed, path)
    big = folder / "ride.built"
    plain = folder / "ride-v1.built"
    with zipfile.ZipFile(path) as src:
        doc = json.loads(src.read("document.json"))
        scene = doc["scene"]
        base = scene["groups"].pop(0)
        template = scene["protos"].pop(base["proto"])
        block = src.read(template["geom"]["member"])
        others = {m: src.read(m) for m in src.namelist()
                  if m != "document.json" and m != template["geom"]["member"]}
    for i in range(DEFINITIONS):
        proto = json.loads(json.dumps(template))
        proto["geom"]["member"] = f"geom/grid-{i}.bin"
        # Each definition its own uid and its own run of ids.
        proto["geom"]["uid"] = f"{i + 1:016x}"
        scene["protos"].append(proto)
        scene["groups"].append(dict(base, proto=i, uid=f"{i + 100:016x}", name=f"Part {i}"))
    for target, strip in ((big, False), (plain, True)):
        d = json.loads(json.dumps(doc))
        if strip:
            d["built_format"] = 1
            for k in ("doc", "requires"):
                d.pop(k, None)
            for g in [d["scene"].get("geom", {})] + [p["geom"] for p in d["scene"]["protos"]]:
                g.pop("uid", None)
                g.pop("ids", None)
                for k in ("vertex_id", "edge_id", "face_id"):
                    g.get("arrays", {}).pop(k, None)
        with zipfile.ZipFile(target, "w", zipfile.ZIP_DEFLATED, compresslevel=1) as dst:
            for m, data in others.items():
                dst.writestr(m, data)
            for i in range(DEFINITIONS):
                dst.writestr(f"geom/grid-{i}.bin", block)
            dst.writestr("document.json", json.dumps(d))
    return big, plain


def _best(fn, n=3):
    times = []
    out = None
    for _ in range(n):
        t = time.perf_counter()
        out = fn()
        times.append(time.perf_counter() - t)
    return min(times), out


@pytest.fixture(scope="module")
def documents(tmp_path_factory):
    return _document(tmp_path_factory.mktemp("ride"))


def _open(path):
    from built_core.scene import Scene
    from formats import igz
    sc = Scene()
    igz.load_into(sc, path)
    return sc


def _touch(sc):
    for g in sc.groups:
        g.mesh.edges
    return sc


def test_ids_cost_almost_nothing_on_an_untouched_ride(documents, record_property):
    big, plain = documents
    size = big.stat().st_size / plain.stat().st_size
    t_ids, sc = _best(lambda: _open(big))
    t_plain, _ = _best(lambda: _open(plain))
    b_ids, _ = _best(lambda: _touch(_open(big)), n=2)
    b_plain, _ = _best(lambda: _touch(_open(plain)), n=2)
    record_property("size_ratio", size)
    record_property("open_ms", (t_ids * 1000, t_plain * 1000))
    record_property("open_and_build_ms", (b_ids * 1000, b_plain * 1000))
    print(f"size ×{size:.4f}  open {t_ids*1e3:.0f} vs {t_plain*1e3:.0f} ms  "
          f"open+build {b_ids*1e3:.0f} vs {b_plain*1e3:.0f} ms")
    assert size < 1.01
    # Gates from §3.6 (open +2 %); 15 ms of slack for timer noise.
    assert t_ids <= t_plain * 1.02 + 0.015
    assert b_ids <= b_plain * 1.05 + 0.05


def test_id_columns_stay_implicit_until_an_edit(documents):
    big, _ = documents
    sc = _touch(_open(big))
    store = sc.groups[0].mesh._store
    for kind in ("face", "edge"):
        assert store.ids_implicit(kind)


def test_save_with_dense_ids_stays_within_budget(documents, tmp_path):
    from built_core import Vec3 as V
    from formats import igz
    big, _ = documents
    sc = _touch(_open(big))
    # Every definition edited once: its new face breaks the implicit run.
    for g in sc.groups:
        g.mesh.add_face([V(-3, -3, 0), V(-2, -3, 0), V(-2, -2, 0)])
    edited = tmp_path / "edited.built"
    t, _ = _best(lambda: igz.save_scene(sc, edited), n=1)
    growth = edited.stat().st_size / big.stat().st_size
    print(f"dense save {t*1e3:.0f} ms, file ×{growth:.4f}")
    # Delta-coded columns: well under one byte per element.
    assert growth < 1.05
