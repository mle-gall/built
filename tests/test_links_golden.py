# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Linked files must answer as ``core/links.py``'s Python did before its
logic moved to Rust.

``tests/fixtures/links_golden.json.gz`` holds, for seeded source files and
seeded runs of the link operations (link, copy, save and reopen, the source
edited and reloaded, host switches and overrides, packing, the nested-link
policy, Unlink with its undo and redo, subsets), the host's state after
each step: its tags and tag folders, the links registry, the materials, the
placement tree with every part's tag, paint, matrix, link record and owner,
and each mesh's faces and edges (tag and paint per element, read from a
deferred mesh's tables without building it), plus what the queries answer
(``sources``, ``linked_groups``, ``linked_ancestor``, ``source_of_tag``,
``query``, ``link_state``, ``is_packed``, ``save_as_placeholder``).

Uids are random, so they are compared by order of first appearance (with,
for each linked part, whether its uid is ``stable_uid`` of its link's and
its source uid); temporary paths read as ``<tmp>``; a load time reads as
present. Regenerate only on purpose, against the reference implementation:
``python tests/test_links_golden.py --write``."""
from __future__ import annotations

import gzip
import json
import os
import random
import sys
import tempfile
from pathlib import Path

GOLDEN = Path(__file__).parent / "fixtures" / "links_golden.json.gz"
CASES = 60

TAGS = ["Wood", "Steel", "Beams", "Glass", "Walls", "Roof", "Notes", "Ü"]
FOLDERS = ["Structure", "Steel", "Envelope", "Annex", "Linked files"]
MATS = ["Oak", "Brick", "Glass", "Concrete"]


def _fn(name):
    """A links operation by its Python name: the native ``link_*`` function
    when there is one (after the move), else ``built_core.links``'s."""
    import built_core
    from built_core import links
    for cand in ("link_" + name, name):
        f = getattr(built_core, cand, None)
        if f is not None and cand.startswith("link"):
            return f
    return getattr(links, name)


def _registry(scene):
    import built_core
    from built_core import links
    read = getattr(built_core, "link_registry_view", None)
    return read(scene) if read is not None else _fn("registry_view")(scene)


class _Canon:
    def __init__(self):
        self.seen = {}

    def __call__(self, v):
        if v is None:
            return None
        return self.seen.setdefault(v, len(self.seen))


def _clean(v, tmp):
    if isinstance(v, str):
        for t in tmp:
            v = v.replace(t, "<tmp>")
        return v
    if isinstance(v, dict):
        return {_clean(k, tmp): _clean(x, tmp) for k, x in v.items()}
    if isinstance(v, (list, tuple)):
        return [_clean(x, tmp) for x in v]
    if isinstance(v, float):
        return round(v, 9)
    return v


def _mesh_rows(mesh):
    """Each face's (tag, front paint, back paint, corner count) and each
    edge's tag, in order; a deferred mesh's from its tables, unbuilt."""
    from built_core.mesh import DeferredMesh
    from built_core.layers import layer_of

    def paint(a):
        a = a or {}
        back = a.get("back")
        return (a.get("layer") or "Layer 0", a.get("mat"),
                back.get("mat") if isinstance(back, dict) else back)
    if type(mesh) is DeferredMesh:
        import numpy as np
        src = mesh.__dict__["_deferred_source"]
        meta = src.get("meta") or {}
        table = meta.get("face_attrs") or []
        fa = src.get("face_attr")
        fa = [] if fa is None else np.asarray(fa).tolist()
        counts = src.get("counts")
        counts = [] if counts is None else np.asarray(counts).tolist()
        faces = [paint(table[k] if k >= 0 else None) + (int(c),) for k, c in zip(fa, counts)]  # (tag, paint, back, rings)
        names = meta.get("edge_layers") or []
        lay = src.get("layers")
        n = len(src["edges"]) if src.get("edges") is not None else 0
        lay = [-1] * n if lay is None else np.asarray(lay).tolist()
        edges = [(names[k] if k >= 0 else None) or "Layer 0" for k in lay]
        return {"deferred": True, "faces": faces, "edges": edges}
    faces = [paint(dict(f.attrs or {}, layer=layer_of(f))) + (1 + len(getattr(f, "holes", None) or ()),)
             for f in mesh.faces]
    edges = [layer_of(e) for e in mesh.edges]
    return {"deferred": False, "faces": faces, "edges": edges}


def _built_rows(mesh):
    from built_core.layers import layer_of
    out = []
    for f in mesh.faces:
        a = {k: v for k, v in (f.attrs or {}).items() if k != "layer"}
        out.append(("f", layer_of(f), repr(sorted(a.items())),
                    [tuple(round(c, 6) for c in (v.x(), v.y(), v.z())) for v in f.vertices]))
    for e in mesh.edges:
        out.append(("e", layer_of(e), bool(getattr(e, "soft", False)),
                    bool(getattr(e, "hidden", False))))
    return out


def fingerprint(scene, tmp, *, build=False):
    from built_core import links
    uids, meshes, fuids = _Canon(), _Canon(), _Canon()
    mesh_rows = {}
    out = {}
    out["tags"] = [(ly.key, ly.name, ly.folder, bool(ly.visible), repr(ly.color),
                    ly.linetype, ly.lineweight, ly.opacity, bool(ly.locked))
                   for ly in scene.layers]
    fuid = dict(scene.layer_folder_uids)
    names_of_uid = {u: n for n, u in fuid.items()}
    out["folders"] = [list(scene.layer_folders), sorted(scene.folder_parents.items(), key=repr),
                      sorted(scene.folder_visibility.items(), key=repr),
                      sorted(scene.layer_folder_names.items(), key=repr),
                      sorted((n, fuids(u)) for n, u in fuid.items())]
    reg = {}
    for key, entry in _registry(scene).items():
        e = dict(entry) if isinstance(entry, dict) else entry
        if isinstance(e, dict):
            if "loaded_mtime_ns" in e:
                e["loaded_mtime_ns"] = True
            if "folder_uids" in e:
                e["folder_uids"] = sorted(names_of_uid.get(u, "?") for u in e["folder_uids"])
        reg[key] = e
    out["registry"] = _clean(reg, tmp)
    out["plugin_data"] = _clean({k: v for k, v in (scene.plugin_data or {}).items()
                                 if k != "links"}, tmp)
    out["materials"] = sorted((n, repr(m.color), repr(getattr(m, "pbr", None)))
                              for n, m in scene.materials.items())
    out["errors"] = _clean(dict(getattr(scene, "link_errors", None) or {}), tmp)

    def node(g, parent, link_uid):
        src_uid = getattr(g, "source_uid", None)
        owner = g.owner
        rec = {
            "uid": uids(g.uid), "name": g.name, "layer": g.layer,
            "source_uid": uids(src_uid),
            "stable": (src_uid is not None and link_uid is not None
                       and g.uid == _fn("stable_uid")(link_uid, src_uid)),
            "owner": ("parent" if owner is parent and owner is not None
                      else None if owner is None else "other"),
            "component": bool(g.component), "hidden": bool(g.hidden),
            "locked": bool(g.locked),
            "material": repr(g.material),
            "xform": None if g.xform is None else [round(v, 9) for v in g.xform.data()],
            "ext": _clean(g.ext, tmp), "ifc": _clean(g.ifc, tmp),
            "mesh": meshes(id(g.mesh)),
        }
        if id(g.mesh) not in mesh_rows:
            mesh_rows[id(g.mesh)] = (_built_rows if build else _mesh_rows)(g.mesh)
        inner = g.uid if _fn("is_linked")(g) else link_uid
        rec["children"] = [node(c, g, inner) for c in g.children or ()]
        return rec
    out["tree"] = [node(g, None, None) for g in scene.groups]
    out["meshes"] = [mesh_rows[k] for k in sorted(mesh_rows, key=meshes)]
    # The queries.
    from built_core.purge import iter_groups
    q = {}
    q["sources"] = _clean(_fn("sources")(scene), tmp)
    q["linked"] = [uids(g.uid) for g in _fn("linked_groups")(scene)]
    q["by_path"] = {_clean(p, tmp): [uids(g.uid) for g in _fn("linked_groups")(scene, p)]
                    for p in _fn("sources")(scene)}
    q["ancestor"] = [uids(getattr(_fn("linked_ancestor")(g), "uid", None))
                     for g in iter_groups(scene.groups)]
    q["placeholder"] = [_fn("save_as_placeholder")(g, scene) for g in iter_groups(scene.groups)]
    q["tag_source"] = [_clean(_fn("source_of_tag")(scene, ly.key), tmp) for ly in scene.layers]
    q["packed"] = [_fn("is_packed")(scene, p) for p in _fn("sources")(scene)]
    q["state"] = [_fn("link_state")(scene, p) for p in _fn("sources")(scene)]
    rows = []
    for r in _fn("query")(scene):
        r = dict(r)
        r["uid"] = uids(r["uid"])
        rows.append(_clean(r, tmp))
    q["query"] = rows
    out["queries"] = q
    return out


# ---- the corpus -----------------------------------------------------------------

def _V(x, y, z=0.0):
    from built_core import Vec3
    return Vec3(float(x), float(y), float(z))


def _square(mesh, x, y, z, rng):
    return mesh.add_face([_V(x, y, z), _V(x + 1, y, z), _V(x + 1, y + 1, z), _V(x, y + 1, z)])


def _paint(f, rng, mats):
    if mats and rng.random() < 0.5:
        f.attrs.update(mats[rng.randrange(len(mats))].face_attrs())
    if mats and rng.random() < 0.15:
        back = mats[rng.randrange(len(mats))]
        f.attrs["back"] = dict(back.face_attrs())


def _write_source(path, rng, *, nested=None, generation=0):
    """A seeded source: tags (some with keys of their own) in a folder
    tree, faces on them or untagged, materials (some colliding with the
    host's), groups and a component placed twice; optionally a file
    linked inside it (``nested``)."""
    from built_core import links
    from built_core.group import Group
    from built_core.layers import Layer, DEFAULT_LAYER
    from built_core.materials import Material
    from built_core.scene import Scene
    from formats import igz
    from built_core import Matrix4
    s = Scene()
    folders = rng.sample(FOLDERS, rng.randint(0, 3))
    s.layer_folders = list(folders)
    if len(folders) > 1 and rng.random() < 0.6:
        s.folder_parents = {folders[1]: folders[0]}
    if folders and rng.random() < 0.3:
        s.folder_visibility = {folders[0]: False}
    if folders and rng.random() < 0.3:
        s.layer_folder_names = {folders[-1]: folders[-1] + " label"}
    tags = []
    for name in rng.sample(TAGS, rng.randint(1, 5)):
        kw = {"visible": rng.random() < 0.8}
        if rng.random() < 0.25:
            kw["key"] = f"{name}-k{generation}"
        if rng.random() < 0.5:
            kw["color"] = tuple(round(rng.random(), 2) for _ in range(3))
        if folders and rng.random() < 0.6:
            kw["folder"] = rng.choice(folders)
        if rng.random() < 0.2:
            kw["linetype"] = "dashed"
        ly = Layer(name, **kw)
        s.layers.append(ly)
        tags.append(ly.key)
    choices = tags + [None, None, DEFAULT_LAYER]
    mats = []
    for name in rng.sample(MATS, rng.randint(0, 3)):
        m = s.materials[name] = Material(name, color=(rng.choice((0.1, 0.5, 0.9)), 0.3, 0.2))
        mats.append(m)

    def mesh_of(n, x, y, z):
        from built_core.mesh import Mesh
        m = Mesh()
        for i in range(n):
            f = _square(m, x + i, y, z, rng)
            t = rng.choice(choices)
            if t is not None:
                f.attrs["layer"] = t
            _paint(f, rng, mats)
        if rng.random() < 0.4 and m.edges:
            e = m.edges[0]
            from built_core.layers import assign_layer
            assign_layer(e, rng.choice(tags))
        return m
    for k in range(rng.randint(1, 3)):
        g = Group(mesh_of(rng.randint(1, 3), 3 * k, 0, 0), name=f"part {k}")
        g.xform = Matrix4()
        g.xform.translate(3.0 * k, 0, 0)
        if rng.random() < 0.5:
            g.layer = rng.choice(tags)
        if rng.random() < 0.3:
            g.material = {"color": [0.2, 0.4, 0.6]}
        if rng.random() < 0.4:
            child = Group(mesh_of(1, 0, 5, 0), name=f"sub {k}")
            child.xform = Matrix4()
            if rng.random() < 0.5:
                child.layer = rng.choice(tags)
            g.adopt([child])
        s.groups.append(g)
    if rng.random() < 0.6:
        proto = mesh_of(2, 0, 0, 2)
        parts = []
        for dx in (0.0, 4.0):
            p = Group(proto, name="leg")
            p.xform = Matrix4()
            p.xform.translate(dx, 0, 2)
            p.layer = rng.choice(choices[:-1]) if rng.random() < 0.6 else None
            parts.append(p)
        asm = Group(mesh_of(1, 0, 0, 3), name="asm")
        asm.adopt(parts)
        s.groups.append(asm)
    if rng.random() < 0.7:
        for i in range(rng.randint(1, 2)):
            f = s.mesh.add_face([_V(i, 9, 0), _V(i + 1, 9, 0), _V(i + 1, 10, 0)])
            t = rng.choice(choices)
            if t is not None:
                f.attrs["layer"] = t
            _paint(f, rng, mats)
    if nested is not None:
        s.groups.append(_fn("make_link")(s, nested, host_path=path))
    igz.save_scene(s, path)
    return path


def _host(rng):
    from built_core.layers import Layer
    from built_core.materials import Material
    from built_core.scene import Scene
    h = Scene()
    if rng.random() < 0.6:
        h.layer_folders = ["Structure"]
    for name in rng.sample(TAGS, rng.randint(0, 3)):
        h.layers.append(Layer(name, folder="Structure" if h.layer_folders and rng.random() < 0.5 else None,
                              visible=rng.random() < 0.8))
    for name in rng.sample(MATS, rng.randint(0, 2)):
        h.materials[name] = Material(name, color=(0.9, 0.9, rng.choice((0.2, 0.3))))
    return h


class _Run:
    def __init__(self, seed, tmp):
        self.rng = random.Random(seed)
        self.tmp = Path(tmp)
        self.names = sorted({str(self.tmp), str(self.tmp.resolve())}, key=len, reverse=True)
        self.steps = []
        self.history = None

    def record(self, label, scene, *, build=False):
        self.steps.append([label, fingerprint(scene, self.names, build=build)])


def _top_links(host):
    from built_core import links
    return [g for g in host.groups if _fn("is_linked")(g)]


def run_case(seed, tmp):
    """Every step's fingerprint for case ``seed``."""
    from built_core import links
    from built_core.group import copy_group
    from built_core.history import History, UnlinkCommand
    from built_core.scene import Scene
    from formats import igz
    import built_core
    run = _Run(seed, tmp)
    rng = run.rng
    tmp = run.tmp
    leaf = None
    if rng.random() < 0.35:
        leaf = _write_source(tmp / "Leaf.igz", rng, generation=9)
    srcs = [_write_source(tmp / "Site.igz", rng, nested=leaf)]
    if rng.random() < 0.4:
        srcs.append(_write_source(tmp / "Annex.igz", rng))
    host = _host(rng)
    host_path = tmp / "host.igz"
    history = History(host)

    def link(src):
        sel = None
        nested = True
        r = rng.random()
        if r < 0.12:
            sel = {"uids": [g.uid for g in _src_groups(src)[:1]]}
        elif r < 0.2:
            nested = False
        g = _fn("make_link")(host, src, host_path=host_path, select=sel,
                            include_nested=nested)
        host.groups.append(g)
        host.version += 1
        return g
    for src in srcs:
        link(src)
    run.record("link", host)
    ops = ["copy", "reopen", "edit", "switch", "note", "clear", "pack", "nested",
           "unlink", "undo", "redo", "relink", "query", "build"]
    for step in range(rng.randint(4, 8)):
        op = rng.choice(ops)
        tops = _top_links(host)
        try:
            if op == "copy" and tops:
                host.groups.append(copy_group(rng.choice(tops), _V(10, 0)))
            elif op == "reopen":
                igz.save_scene(host, host_path)
                host = Scene()
                igz.load_into(host, host_path)
                history = History(host)
            elif op == "edit":
                src = rng.choice(srcs)
                _write_source(src, rng, nested=leaf if src == srcs[0] else None,
                              generation=step)
                _fn("reload")(host, src, host_path)
            elif op == "switch" and host.layers:
                ly = rng.choice(list(host.layers))
                built_core.tag_edits.set_visible(host, [ly.key], not ly.visible)
            elif op == "note" and host.layers:
                ly = rng.choice(list(host.layers))
                run.steps.append(["note-answer", _fn("note_visibility")(host, ly.key, False)])
            elif op == "clear" and _fn("sources")(host):
                _fn("clear_overrides")(host, rng.choice(_fn("sources")(host)), host_path)
            elif op == "pack" and _fn("sources")(host):
                p = rng.choice(_fn("sources")(host))
                _fn("set_packed")(host, p, not _fn("is_packed")(host, p))
            elif op == "nested" and tops:
                g = rng.choice(tops)
                _fn("set_include_nested")(host, g, not _fn("link_of")(g).get("include_nested", True))
            elif op == "unlink" and tops:
                history.execute(UnlinkCommand([rng.choice(tops)]))
            elif op == "undo":
                history.undo()
            elif op == "redo":
                history.redo()
            elif op == "relink":
                link(rng.choice(srcs))
            elif op == "build":
                from built_core.group import iter_placements
                for g in host.groups:
                    for p, _m in iter_placements(g):
                        len(p.mesh.faces)
            else:
                op = "skip"
        except Exception as exc:  # noqa: BLE001 — the answer is the error
            run.steps.append([op + "-error", type(exc).__name__, _clean(str(exc), run.names)])
        run.record(op, host)
    run.record("built", host, build=True)
    # A missing source, then a cycle.
    if rng.random() < 0.5 and srcs[1:]:
        igz.save_scene(host, host_path)
        os.replace(srcs[1], tmp / "gone.igz")
        back = Scene()
        igz.load_into(back, host_path)
        run.record("missing", back)
    return run.steps


def _src_groups(path):
    from built_core.scene import Scene
    from formats import igz
    s = Scene()
    igz.load_into(s, path)
    return list(s.groups)


def pure_cases():
    """The path and name helpers, on fixed inputs."""
    from built_core import links
    out = {}
    out["stable_uid"] = [_fn("stable_uid")(a, b) for a, b in
                         [("a", "b"), ("", ""), ("0123456789abcdef", "fedcba9876543210"),
                          ("Ü", "日本"), ("x" * 200, "y:z")]]
    out["tag_name"] = [_fn("tag_name")(p, t) for p in ("/a/Site.igz", "Site", "/a/b.c.igz", "/a/.hidden")
                       for t in (None, "", "Layer 0", "Wood", "Mid / Steel")]
    errors = []
    for sel in (None, {}, {"uids": []}, {"uids": ["a"]}, {"uids": [["a", "b"]]},
                {"uids": [[]]}, {"uids": "a"}, {"uids": [1]}, {"uids": ["a"], "filter": {}},
                {"other": 1}, [1]):
        try:
            _fn("validate_select")(sel)
            errors.append(None)
        except Exception as exc:  # noqa: BLE001
            errors.append([type(exc).__name__, str(exc)])
    out["validate_select"] = errors
    return out


def _with_uids(fn):
    """Seeded group and folder uids, so a run is the same every time."""
    import built_core.group as group
    import built_core.layers as layers
    import itertools
    counter = itertools.count()
    old_g, old_l = group.new_uid, getattr(layers, "new_uid", None)
    group.new_uid = lambda: f"u{next(counter):06d}"
    layers.new_uid = lambda: f"f{next(counter):06d}"
    try:
        return fn()
    finally:
        group.new_uid = old_g
        if old_l is not None:
            layers.new_uid = old_l


def generate():
    out = {"pure": pure_cases(), "cases": []}
    for seed in range(CASES):
        with tempfile.TemporaryDirectory() as tmp:
            out["cases"].append(_with_uids(lambda: run_case(seed, tmp)))
    return json.loads(json.dumps(out, default=repr))


def _golden():
    with gzip.open(GOLDEN, "rt") as f:
        return json.load(f)


def test_pure_helpers():
    assert json.loads(json.dumps(pure_cases(), default=repr)) == _golden()["pure"]


def _diff(a, b, path="$"):
    if type(a) is not type(b):
        return f"{path}: {a!r:.300} != {b!r:.300}"
    if isinstance(a, dict):
        for k in sorted(set(a) | set(b)):
            if k not in a or k not in b:
                return f"{path}.{k}: missing on one side"
            d = _diff(a[k], b[k], f"{path}.{k}")
            if d:
                return d
        return None
    if isinstance(a, list):
        if len(a) != len(b):
            return f"{path}: {len(a)} != {len(b)} items: {a!r:.400} != {b!r:.400}"
        for i, (x, y) in enumerate(zip(a, b)):
            d = _diff(x, y, f"{path}[{i}]")
            if d:
                return d
        return None
    return None if a == b else f"{path}: {a!r:.300} != {b!r:.300}"


import pytest  # noqa: E402


@pytest.mark.parametrize("seed", range(CASES))
def test_link_runs(seed, tmp_path):
    want = _golden()["cases"][seed]
    got = json.loads(json.dumps(_with_uids(lambda: run_case(seed, tmp_path)), default=repr))
    assert _diff(got, want) is None, _diff(got, want)


if __name__ == "__main__":
    if "--write" in sys.argv:
        data = generate()
        GOLDEN.parent.mkdir(exist_ok=True)
        raw = json.dumps(data, sort_keys=True).encode()
        with open(GOLDEN, "wb") as fh:
            with gzip.GzipFile(fileobj=fh, mode="wb", mtime=0) as gz:
                gz.write(raw)
        n = sum(len(c) for c in data["cases"])
        print(f"wrote {GOLDEN} ({n} steps)")
