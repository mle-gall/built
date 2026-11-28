# SPDX-License-Identifier: GPL-3.0-or-later
"""Empty-document arrangements for every starter junction detail: the
members each detail's roles need, drawn the way a user would draw them."""
from tests import addon_cases
from built_core import Matrix4, Vec3
from built_core import command_protocol as live
from built_core.group import Group
from built_core.history import History
from built_core.scene import Scene
from tests.test_geometry_arrays import CUBE
from tests.test_live_recipes import apply
from tests.test_sketch_rows import member

STARTER_PACKS = ("Steel connections · EN", "Timber joints", "Generic", "Architecture", "Purlin connections · EN", "MEP penetrations")
HE, IPE, TIMBER, FLAT, WALL = (addon_cases.profile(k) for k in ("en/he200a", "en/ipe240", "basic/timber", "basic/flat", "basic/wall"))


def box(scene, name, cls, lo, hi):
    g = Group(name=name)
    g.xform = Matrix4()
    g.ifc = {"class": cls}
    for face in CUBE:
        g.mesh.add_face([Vec3(*(lo[i] + (hi[i] - lo[i]) * c for i, c in enumerate(p))) for p in face])
    scene.groups.append(g)
    return g


def sweeps(h, *specs, **kw):
    scene = h.scene
    before = len(scene.groups)
    apply(h, "create_sweeps", sweeps=[member(p, pts) for p, pts in specs], **kw)
    return [g.uid for g in scene.groups[before:]]


def hosted(scene, h, detail_uid, cls):
    """A wall with a door or window placed on it, as the Wall tool does."""
    apply(h, "create_sweeps", sweeps=[dict(profile=dict(library="basic/wall", params=dict(width=200, height=3000)),
          attachment="bottom_left", path=dict(points=[[0, 0, 0], [6, 0, 0]]))])
    wall = scene.groups[-1]
    fill = Group(name=cls[3:])
    fill.xform = Matrix4()
    fill.ifc = {"class": cls}
    fill.ext = {"library_host": "wall", "junction": detail_uid}
    for face in CUBE:
        fill.mesh.add_face([Vec3(x, .12 * y, z) for x, y, z in face])
    apply(h, "place_group", template=fill.uid, _pending=fill, basis=[[1, 0, 0], [0, 1, 0], [0, 0, 1]], offset=[2, 0, 1],
          wall_host=dict(uid=wall.uid, outline=[[2, 0, 1], [3, 0, 1], [3, 0, 2], [2, 0, 2]],
                         normal=[0, -1, 0], initial_offset=0, offset=0, sill=1, flipped=False))


def arrangement(scene, h, p):
    """The picks (main first) for a detail, after drawing its members."""
    name = p["name"]
    meeting = p["values"]["meeting"]
    if meeting == "hosted":
        return None
    if name in ("End plate, 4 bolts", "Fin plate"):
        return sweeps(h, (HE, [[0, 0, 0], [0, 0, 3]]), (IPE, [[0, 0, 2], [0, 2, 2]]))
    if name in ("Base plate", "Wall on slab", "Wall on slab · trim"):
        slab = box(scene, "Slab", "IfcSlab", (-1, -1, 0), (3, 3, .25))
        if name == "Base plate":
            return [slab.uid] + sweeps(h, (HE, [[0, 0, .25], [0, 0, 3]]))
        z = 0 if name.endswith("trim") else .25
        apply(h, "create_sweeps", sweeps=[dict(profile=WALL, attachment="bottom_left", path=dict(points=[[0, 0, z], [2, 0, z]]))])
        return [slab.uid, scene.groups[-1].uid]
    if name == "Purlin cleat":
        return sweeps(h, (HE, [[-1.5, 0, 0], [1.5, 0, 0]]),
                      (IPE, [[-.4, -1, .215], [.4, 1, .215]]))
    if p["pack"] == "MEP penetrations":
        profile = {"library": "basic/rail" if name.startswith("Pipe") else "basic/duct"}
        if name.endswith("wall"):
            return sweeps(h, (WALL, [[-1, 0, 0], [1, 0, 0]]),
                          (profile, [[0, -1, .3], [0, 1, .3]]))
        slab = box(scene, "Slab", "IfcSlab", (-1, -1, 0), (1, 1, .25))
        return [slab.uid] + sweeps(h, (profile, [[0, 0, -.3], [0, 0, .6]]))
    if name == "Apex haunch":
        return sweeps(h, (IPE, [[-2, 0, 0], [0, 0, .5]]), (IPE, [[0, 0, .5], [2, 0, 0]]))
    if name == "Birdsmouth":
        return sweeps(h, (TIMBER, [[0, -1, 0], [0, 1, 0]]), (TIMBER, [[-.5, 0, -.035], [1.5, 0, .505]]))
    if name == "Half-lap":
        return sweeps(h, (TIMBER, [[0, -1, 0], [0, 1, 0]]), (TIMBER, [[-1, 0, 0], [1, 0, 0]]))
    if name == "Tenon and mortise":
        return sweeps(h, ({"library": "basic/timber", "params": {"width": 145}}, [[0, 0, 0], [0, 0, 2]]),
                      (TIMBER, [[0, 0, 1], [1, 0, 1]]))
    if name in ("Joist hanger", "Butt with screws"):
        return sweeps(h, (TIMBER, [[0, -1, 0], [0, 1, 0]]), (TIMBER, [[0, 0, 0], [2, 0, 0]]))
    if name in ("Corner mitre",):
        return sweeps(h, (TIMBER, [[0, 0, 0], [2, 0, 0]]), (TIMBER, [[2, 0, 0], [2, 3, 0]]))
    if name in ("Wall corner · mitre", "Wall corner · by priority"):
        return sweeps(h, (WALL, [[0, 0, 0], [2, 0, 0]]), (WALL, [[2, 0, 0], [2, 1, 0]]))
    if name == "Wall tee":
        return sweeps(h, (WALL, [[0, 0, 0], [2, 0, 0]]), (WALL, [[1, 0, 0], [1, 1, 0]]))
    if meeting == "parts":
        plate = next(q for q, _ in live.preset_catalog(scene) if q["name"] == "Plate")
        if name == "Fillet weld":
            # A column standing on a plate lying flat.
            flat = sweeps(h, ({"library": "basic/flat", "params": {"width": 300, "height": 12}}, [[-.15, 0, .006], [.15, 0, .006]]))
            return sweeps(h, (HE, [[0, 0, .012], [0, 0, 1]])) + flat
        rows = []
        for z in (0, .012):
            apply(h, "create_sweeps", sweeps=[dict(profile=plate["values"]["profile"], preset_uid=plate["uid"],
                  path=dict(points=[[0, 0, z], [0, 0, z + .012]]))])
            rows.append(scene.groups[-1].uid)
        return rows
    raise AssertionError(f"No arrangement for {name}")


def starter_catalog(scene=None):
    catalog = {p["uid"]: p for p, _ in live.preset_catalog(scene)}
    for pack in ("steel-connections-en", "timber-joints", "purlin-connections-en", "mep-penetrations"):
        catalog.update((p["uid"], p) for p in addon_cases.rows(pack))
    return list(catalog.values())


def starter_details(scene=None):
    return [p for p in starter_catalog(scene) if p["primitive"] == "junction" and p["pack"] in STARTER_PACKS and p["name"] not in ("Bolt", "Bolt group")]


def apply_in_empty_document(p, embed=()):
    """Apply one starter detail in a new document; returns (scene, history, junction uid)."""
    scene = Scene()
    h = History(scene)
    if embed:
        apply(h, "preset_update", rows=list(embed))
    if p["values"]["meeting"] == "hosted":
        cls = {"Door opening": "IfcDoor"}.get(p["name"], "IfcWindow")
        hosted(scene, h, p["uid"], cls)
        uid = next(r["uid"] for r in live.live_rows(scene) if r["kind"] == "junction")
        return scene, h, uid
    picks = arrangement(scene, h, p)
    definitions = live.junction_definitions(p, starter_catalog(scene))
    bindings = live.junction_bind(scene, p, picks)
    uid = apply(h, "junction_apply", detail=p["uid"], bindings=bindings, definitions=definitions)["uid"]
    return scene, h, uid
