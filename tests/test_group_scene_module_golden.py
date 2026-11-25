# SPDX-License-Identifier: GPL-3.0-or-later
"""``core.group``, ``core.scene`` and ``core.links`` must answer as their
Python files did at ``5c6b50c1``.

For ``core.group`` that covers:
- frames of skewed, scaled and mirrored matrices, and frames from points;
- oriented bounds and box corners;
- a billboard group;
- copies of classic groups, instances and nested components, with their
  IFC guids and openings remapped;
- ``repr``, paint inheritance, placement points and the uid form;
- the field tuples and the class layout (``__slots__``, public bases,
  module).

For ``core.scene``: the dataclass's fields, in order, with their defaults
and flags, then a scene's visibility, selectability and selection rules.

For ``core.links``: the constants, ``LinkError`` and the registry step.

Matrices and vectors read as rounded tuples. The golden is
``tests/fixtures/group_scene_module_golden.json``. Regenerate it only on
purpose: ``python tests/test_group_scene_module_golden.py --write``."""
from __future__ import annotations

import json
import re
import sys
from pathlib import Path

GOLDEN = Path(__file__).parent / "fixtures" / "group_scene_module_golden.json"


def _r(x):
    return round(float(x), 9) + 0.0


def vec(v):
    return [_r(v.x()), _r(v.y()), _r(v.z())]


def mat(m):
    return None if m is None else [_r(x) for x in m.data()]


def answers():
    import dataclasses

    import numpy as np
    from built_core import Matrix4 as M, Vec3 as V
    from built_core.mesh import Mesh

    import core.group as G
    import core.links as L
    from core.scene import Scene

    out = {}
    frames = []
    for build in ([], [("rotate", 30, 0, 0, 1)], [("scale", 2, 3, 0.5)], [("scale", -1, 1, 1)],
                  [("translate", 1, 2, 3), ("rotate", 45, 1, 1, 0), ("scale", 1, 0, 1)]):
        m = M()
        for op, *args in build:
            getattr(m, op)(*args)
        frames.append([vec(a) for a in G.frame_axes(m)])
    frames.append([vec(a) for a in G.frame_axes(None)])
    out["frame_axes"] = frames
    pts = np.array([[0, 0, 0], [2, 0, 0], [2, 1, 0], [0, 1, 0], [1, 3, 2]], dtype=float)
    rot = np.array([[np.cos(0.4), -np.sin(0.4), 0], [np.sin(0.4), np.cos(0.4), 0], [0, 0, 1]])
    out["frame_from_points"] = [[vec(a) for a in G.frame_from_points(p)]
                                for p in (pts, pts @ rot.T, pts[:2], None)]
    mesh = Mesh()
    mesh.add_face([V(0, 0, 0), V(3, 0, 0), V(3, 2, 0), V(0, 2, 0)])
    f, lo, hi = G.oriented_bounds(mesh)
    out["bounds"] = [[vec(a) for a in f], [_r(x) for x in lo], [_r(x) for x in hi]]
    f2, lo2, hi2 = G.oriented_bounds(None, frame=G.frame_from_points(pts @ rot.T), points=pts @ rot.T)
    out["bounds_points"] = [[_r(x) for x in lo2], [_r(x) for x in hi2],
                            [vec(c) for c in G.oriented_box_corners(f2, lo2, hi2)]]
    out["bounds_empty"] = [[_r(x) for x in b] for b in G.oriented_bounds(Mesh(), points=np.empty((0, 3)))[1:]]

    bb = G.make_billboard_group("/x/tree.png", 4, "Tree", 0.5, V(1, 2, 0))
    face = list(bb.mesh.faces)[0]
    out["billboard"] = [bb.name, bb.billboard, dict(face.attrs["texture"]),
                        sorted(vec(v.position) for v in bb.mesh.vertices)]

    base = Mesh()
    base.add_face([V(0, 0, 0), V(1, 0, 0), V(1, 1, 0)])
    classic = G.Group(base, name="Classic")
    classic.ifc = {"guid": "g0", "class": "IfcWall", "openings": [{"guid": "o1", "group_uid": "fill"}]}
    classic.material = {"color": [1, 0, 0]}
    classic.axes = M()
    copy = G.copy_group(classic, V(5, 0, 0))
    out["copy_classic"] = [copy.name, copy.mesh is base, sorted(vec(v.position) for v in copy.mesh.vertices),
                           mat(copy.axes), copy.ifc.get("guid"), len(copy.ifc["openings"]),
                           sorted(copy.ifc["openings"][0]), copy.ifc["openings"][0]["group_uid"],
                           dict(copy.material), copy.uid != classic.uid, copy.component]
    inner = G.Group(Mesh(), name="Inner")
    comp = G.Group(Mesh(), name="Comp")
    comp.adopt([inner])
    inner.ifc = {"fills": "o9"}
    comp.ifc = {"openings": [{"guid": "o9", "group_uid": inner.uid}]}
    c2 = G.copy_group(comp, V(0, 1, 0))
    kid = c2.children[0]
    out["copy_component"] = [mat(c2.xform), c2.mesh is comp.mesh, kid.mesh is inner.mesh, mat(kid.xform),
                             kid.component, inner.component, mat(inner.xform),
                             c2.ifc["openings"][0]["group_uid"] == kid.uid,
                             kid.ifc["fills"] == c2.ifc["openings"][0]["guid"],
                             bool(re.fullmatch(r"[0-9a-f]{16}", kid.uid))]
    out["repr"] = [repr(classic), repr(comp)]
    child = G.Group(Mesh(), name="c")
    child.owner = comp
    comp.material = {"color": [0, 1, 0]}
    out["paint"] = [G.effective_material(child), G.effective_material(classic), G.effective_material(None)]
    pp = G.placement_points(c2)
    out["placement_points"] = list(pp.shape)
    uid = G.new_uid()
    out["uid"] = [len(uid), uid[12], bool(re.fullmatch(r"[0-9a-f]{16}", uid)), uid != G.new_uid()]
    out["fields"] = [list(G.GROUP_FIELDS), list(G.DOC_FIELDS), list(G.ATTR_FIELDS), list(G._PY_FIELDS)]
    out["classes"] = [G.Group.__module__, G.Group.__name__, list(G.Group.__slots__),
                      [b.__name__ for b in G.Group.__mro__ if not b.__name__.startswith("_")],
                      G.PlacementProxy.__module__, list(G.PlacementProxy.__slots__),
                      [b.__name__ for b in G.PlacementProxy.__mro__ if not b.__name__.startswith("_")]]
    p = G.PlacementProxy(Mesh(), name="part", uid="u1")
    out["proxy"] = [p.name, p.uid, p.billboard, p.hidden, p.locked, p.component, p.xform, list(p.children),
                    p._doc_key]
    g = G.Group(Mesh())
    out["frame"] = [G.group_frame(None), G.group_frame(g), mat(G.group_frame(comp))]
    G.carry_axes(g, M(2, 0, 0, 0, 0, 2, 0, 0, 0, 0, 2, 0, 0, 0, 0, 1))
    out["carry"] = mat(g.axes)
    out["is_link"] = [G._is_link(g), G._is_link(type("X", (), {"ext": {"link": {"path": "a.igz"}}})())]

    # The scene.
    rows = []
    for fl in dataclasses.fields(Scene):
        d = fl.default if fl.default is not dataclasses.MISSING else None
        fac = fl.default_factory if fl.default_factory is not dataclasses.MISSING else None
        made = fac() if fac is not None and fl.name != "_root" else None
        rows.append([fl.name, repr(d), type(made).__name__ if fac is not None else None,
                     made if isinstance(made, (dict, list)) else None, fl.init, fl.repr, fl.compare])
    out["scene_fields"] = rows
    out["scene_initvars"] = sorted(n for n, t in Scene.__annotations__.items()
                                   if isinstance(t, dataclasses.InitVar) or "InitVar" in str(t))
    s = Scene()
    sq = s.mesh.add_face([V(0, 0, 0), V(1, 0, 0), V(1, 1, 0), V(0, 1, 0)])
    from core.layers import Layer, assign_layer
    s.layers.append(Layer("Walls"))
    s.layers.append(Layer("Gone", visible=False))
    s.layers.append(Layer("Shut", locked=True))
    grp = G.Group(Mesh(), name="G")
    s.groups.append(grp)
    hid = G.Group(Mesh(), name="H")
    hid.hidden = True
    s.groups.append(hid)
    lock = G.Group(Mesh(), name="L")
    lock.locked = True
    s.groups.append(lock)
    edges = list(s.mesh.edges)
    assign_layer(edges[0], "Gone")
    assign_layer(edges[1], "Shut")
    rules = []
    for ent in [sq, *edges, grp, hid, lock]:
        rules.append([s.entity_visible(ent), s.entity_selectable(ent), s.entity_hidden(ent),
                      list(s._layer_state(ent)), Scene.entity_locked(ent), s.edge_snappable(ent),
                      s.group_snappable(ent)])
    out["scene_rules"] = rules
    out["scene_layers"] = [s.layer("Walls").name, s.layer("Nope"), s.layer(""), s.layer_by_key("Gone").key]
    v0 = s.version
    s.select([sq])
    s.select(edges[:2], mode="toggle")
    s.select([sq], mode="remove")
    n = s.invert_selection()
    out["scene_select"] = [n, len(s.selection), s.version - v0, s.view_version, s.content_version,
                           s.geometry_version, type(s.selection).__name__]
    s.clear_selection()
    s.bump_sections()
    out["scene_versions"] = [s.version, s.view_version, s.section_version, s.content_version, s.geometry_version]
    out["scene_place"] = [[x.name, mat(m)] for x, m in s.placements()]
    out["scene_render"] = [len(list(s.render_edges())), len(list(s.render_faces()))]
    out["scene_misc"] = [s.drawing_frame, s.loose_mesh is s.mesh, s.edit_definition(), s.active_section(),
                         sorted(s.groups_by_uid()) == sorted(x.uid for x in s.groups),
                         s.navigator_key(grp) == grp.uid, s.open_group_paint(), Scene.__module__,
                         Scene.__hash__ is None, Scene() == Scene()]
    out["scene_tables"] = [s.units, s.dimension_style, s.show_section_planes, s.active_layer]

    # The links surface.
    out["links"] = [L.LINK_KEY, L.REGISTRY_KEY, L.LINKED_FOLDER, L.SEP, L.LinkError.__module__,
                    issubclass(L.LinkError, Exception)]
    s.link_registry = {"a": 1}
    step = L.set_registry_command({"b": [2]})
    step.do(s)
    after = dict(s.link_registry)
    step.undo(s)
    out["link_step"] = [type(step).__name__, step.doc_undo, after, dict(s.link_registry)]
    return json.loads(json.dumps(out, default=repr))


def test_group_scene_and_links_answer_as_the_python_did():
    assert answers() == json.loads(GOLDEN.read_text())


if __name__ == "__main__":
    if "--write" in sys.argv:
        GOLDEN.write_text(json.dumps(answers(), indent=1))
        print("wrote", GOLDEN)
