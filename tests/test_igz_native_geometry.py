# SPDX-License-Identifier: GPL-3.0-or-later
"""The Rust reader of format-3 ``geom/`` members gives what Python gives.

``ingetrazo_core.igz_read_geometry`` inflates the members in parallel and
checks each block against its layout; ``formats.igz`` then builds the
arrays without scanning them again. The promise is the engine's usual one:
faster, never different — the same bytes, the same arrays, and for a
damaged document the same error as the Python reader."""
from __future__ import annotations

import json
import random
import zipfile

import pytest
from ingetrazo_core import Vec3 as V

from core import _native
from core.group import Group
from core.scene import Scene
from formats import igz

pytestmark = pytest.mark.skipif(
    getattr(_native.available(), "igz_read_geometry", None) is None,
    reason="native module without igz_read_geometry")


@pytest.fixture
def binary(monkeypatch):
    """Every document counts as large and every mesh goes binary."""
    monkeypatch.setattr(igz, "_LARGE_DOCUMENT", 0)
    monkeypatch.setattr(igz, "_BULK_MIN", 1)


@pytest.fixture
def engine(monkeypatch):
    """``engine(True)`` / ``engine(False)``: the reader with / without Rust."""
    mod = _native.available()

    def use(on):
        monkeypatch.setattr(_native, "native", mod if on else None)
    return use


def _scene() -> Scene:
    """Meshes with everything a block can hold: holes, face attributes
    shared by several faces, soft/hidden edges, curves and edge tags."""
    rng = random.Random(20261113)
    scene = Scene()

    def fill(m, n, ox=0.0):
        for i in range(n):
            x, y = ox + (i % 20) * 3.0, (i // 20) * 3.0
            ring = [V(x, y, 0), V(x + 2, y, 0), V(x + 2, y + 2, 0),
                    V(x, y + 2, 0)]
            holes = ([[V(x + .5, y + .5, 0), V(x + 1.5, y + .5, 0),
                       V(x + 1.5, y + 1.5, 0), V(x + .5, y + 1.5, 0)]]
                     if i % 7 == 0 else None)
            f = m.add_face(ring, holes)
            k = rng.randrange(4)
            if k == 1:
                f.attrs.update({"color": [0.5, 0.4, 0.3], "mat": "Madera"})
            elif k == 2:
                f.attrs.update({"layer": "Muros", "opacity": 0.5,
                                "back": {"mat": "Yeso"}})
        for i in range(n // 2):
            e = m.add_edge(V(ox + i, -5, 0), V(ox + i + 0.5, -6, 1))
            e.soft, e.hidden = i % 2 == 0, i % 3 == 0
            if i % 4 == 0:
                e.curve = 10 + i // 8
            if i % 5 == 0:
                e.layer = "Guias"

    fill(scene.mesh, 200)
    for j in range(3):
        g = Group(name=f"Pieza {j}")
        fill(g.mesh, 40 + j * 10, ox=100.0 * (j + 1))
        scene.groups.append(g)
    return scene


def _document(tmp_path, name="doc.igz"):
    path = tmp_path / name
    igz.save_scene(_scene(), path)
    with zipfile.ZipFile(path) as zf:
        assert sum(n.startswith("geom/") for n in zf.namelist()) == 4
    return path


def _read(path, on, engine):
    engine(on)
    with zipfile.ZipFile(path) as zf:
        payload = json.loads(zf.read("document.json"))["scene"]
        members = igz._read_geometry_members(zf, path, payload)
    metas = [b["geom"] for b in igz._mesh_blocks(payload) if "geom" in b]
    return members, [igz._document_geometry_arrays(m, members)
                     for m in metas]


def _same_arrays(a, b):
    assert list(a) == list(b)
    for k in a:
        x, y = a[k], b[k]
        assert (x.dtype, x.shape, x.flags.writeable) == \
            (y.dtype, y.shape, y.flags.writeable), k
        assert x.tobytes() == y.tobytes(), k


def test_members_and_arrays_equal_the_python_reader(tmp_path, binary, engine):
    path = _document(tmp_path)
    py_members, py_arrays = _read(path, False, engine)
    nat_members, nat_arrays = _read(path, True, engine)
    assert dict(py_members) == dict(nat_members)
    assert not py_members.checked
    assert len(nat_members.checked) == len(nat_members)   # all vouched for
    for a, b in zip(py_arrays, nat_arrays):
        _same_arrays(a, b)


def test_parallel_read_keeps_the_inflater_off_the_recursive_worker_stack(tmp_path):
    import os
    import subprocess
    import sys
    import textwrap

    path = tmp_path / "members.zip"
    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as archive:
        archive.writestr("g", b"block")
    # Reuse one compressed range to exercise a deep job tree without a
    # large fixture. The pool reads its stack settings once per process.
    probe = textwrap.dedent("""
        import os, sys, zipfile
        from ingetrazo_core import igz_read_geometry

        with zipfile.ZipFile(sys.argv[1]) as archive:
            member = archive.getinfo("g")
            row = (member.header_offset, member.compress_type, member.compress_size,
                   member.file_size, member.CRC, b"g", member.flag_bits)
            count = 2048
            sources = [{}]
            if os.name == "posix":
                sources.append(dict(fd=archive.fp.fileno(), member_offsets=[31] * count))
            for source in sources:
                rows = igz_read_geometry(sys.argv[1], [row] * count, [None] * count, **source)
                assert rows == [(b"block", False)] * count
        """)
    subprocess.run([sys.executable, "-c", probe, str(path)], check=True,
                   env=dict(os.environ, RUST_MIN_STACK="262144", RAYON_NUM_THREADS="8"))


def test_a_stored_document_reads_the_same(tmp_path, binary, engine):
    # zipfile's default is ZIP_STORED: the other method a member can have.
    path = _document(tmp_path)
    with zipfile.ZipFile(path) as zf:
        members = {n: zf.read(n) for n in zf.namelist()}
    with zipfile.ZipFile(path, "w") as zf:
        for n, data in members.items():
            zf.writestr(n, data)
    py_members, py_arrays = _read(path, False, engine)
    nat_members, nat_arrays = _read(path, True, engine)
    assert dict(py_members) == dict(nat_members)
    assert len(nat_members.checked) == len(nat_members)
    for a, b in zip(py_arrays, nat_arrays):
        _same_arrays(a, b)


def _canonical(scene) -> str:
    def mesh(m):
        block = igz._mesh_json(m)
        block["vertices"] = [v.position.toTuple() for v in m.vertices]
        return block
    return json.dumps({"loose": mesh(scene.mesh),
                       "groups": [(g.name, mesh(g.mesh))
                                  for g in scene.groups]},
                      sort_keys=True, default=list)


def test_the_document_opens_the_same(tmp_path, binary, engine):
    path = _document(tmp_path)
    engine(False)
    py = Scene()
    igz.load_into(py, path)
    engine(True)
    nat = Scene()
    igz.load_into(nat, path)
    assert _canonical(nat) == _canonical(py)


def _tamper(path, mutate, method=zipfile.ZIP_DEFLATED):
    with zipfile.ZipFile(path) as zf:
        members = {n: zf.read(n) for n in zf.namelist()}
    doc = json.loads(members["document.json"])
    mutate(doc, members)
    members["document.json"] = json.dumps(doc).encode()
    with zipfile.ZipFile(path, "w", compression=method) as zf:
        for n, data in members.items():
            zf.writestr(n, data)


def _flip_crc(path):
    """Corrupt the stored data of the first geometry member in place."""
    with zipfile.ZipFile(path) as zf:
        info = zf.getinfo(igz._GEOM_PREFIX + "0.bin")
    raw = bytearray(path.read_bytes())
    n = int.from_bytes(raw[info.header_offset + 26:info.header_offset + 28],
                       "little")
    x = int.from_bytes(raw[info.header_offset + 28:info.header_offset + 30],
                       "little")
    at = info.header_offset + 30 + n + x + 3
    raw[at] ^= 0x01
    path.write_bytes(bytes(raw))


def _scene_geom(doc):
    return doc["scene"]["geom"]


DAMAGE = {
    # an index past the last point (a truncated blob)
    "truncated": lambda d, m: m.__setitem__(
        _scene_geom(d)["member"], m[_scene_geom(d)["member"]][:-8]),
    # an edge pointing at a point that does not exist
    "edge-index": lambda d, m: m.__setitem__(
        _scene_geom(d)["member"],
        m[_scene_geom(d)["member"]][:_scene_geom(d)["arrays"]["edges"][2]]
        + (10 ** 6).to_bytes(4, "little")
        + m[_scene_geom(d)["member"]][_scene_geom(d)["arrays"]["edges"][2] + 4:]),
    # a dtype the reader must not build
    "dtype": lambda d, m: _scene_geom(d)["arrays"]["pos"].__setitem__(0, "|O"),
    # an array said to be bigger than the blob
    "too-big": lambda d, m: _scene_geom(d)["arrays"]["pos"].__setitem__(
        1, [10 ** 9, 3]),
    # a face-attribute table shorter than its indices
    "attr-table": lambda d, m: _scene_geom(d).__setitem__("face_attrs", []),
    # the block itself missing
    "missing": lambda d, m: m.pop(_scene_geom(d)["member"]),
}


def _error(path, on, engine):
    engine(on)
    with pytest.raises(Exception) as info:
        igz.load_into(Scene(), path)
    return type(info.value), str(info.value)


@pytest.mark.parametrize("damage", sorted(DAMAGE))
def test_a_damaged_block_fails_the_same_way(tmp_path, binary, engine, damage):
    path = _document(tmp_path)
    _tamper(path, DAMAGE[damage])
    py = _error(path, False, engine)
    assert issubclass(py[0], ValueError)
    assert _error(path, True, engine) == py


def test_a_bad_crc_fails_the_same_way(tmp_path, binary, engine):
    path = _document(tmp_path)
    _flip_crc(path)
    py = _error(path, False, engine)
    assert _error(path, True, engine) == py


def test_an_engine_failure_falls_back_to_python(tmp_path, binary, engine,
                                                monkeypatch):
    path = _document(tmp_path)
    engine(False)
    py = Scene()
    igz.load_into(py, path)

    real = _native.available()

    class Broken:
        """The real engine, whose geometry reader fails."""
        def __getattr__(self, name):
            return getattr(real, name)

        @staticmethod
        def igz_read_geometry(*_a):
            raise RuntimeError("boom")
    monkeypatch.setattr(_native, "native", Broken())
    back = Scene()
    igz.load_into(back, path)
    assert _canonical(back) == _canonical(py)


def test_open_geometry_uses_original_archive_after_atomic_replace(tmp_path, binary):
    import os
    from formats.native_io import DocumentArchive, native_io

    if not hasattr(native_io(), "igz_read_member"):
        pytest.skip("native descriptor reader unavailable")
    path = _document(tmp_path)
    with DocumentArchive(path) as archive:
        payload = json.loads(archive.read("document.json"))["scene"]
        old = {name: archive.read(name) for name in archive.namelist()
               if name.startswith("geom/")}
        replacement = tmp_path / "replacement"
        replacement.write_bytes(b"replacement")
        os.replace(replacement, path)
        members = igz._read_geometry_members(archive, path, payload)
    assert members == old
    assert len(members.checked) == len(old)
