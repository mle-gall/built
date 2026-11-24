"""Recorded ordered results from the materialised geometry and snap implementations."""
import hashlib
import json
import random

from ingetrazo_core import Vec3 as V

from core import edits, snap, topology
from core.mesh import Mesh
from core.scene import Scene


def digest(rows):
    return hashlib.sha256(json.dumps(rows, separators=(",", ":")).encode()).hexdigest()


def xyz(p):
    return tuple(round(v, 9) for v in p.toTuple())


def push_rows():
    rows = []
    for seed in range(32):
        rng = random.Random(seed)
        m = Mesh()
        x = rng.choice([0, 650000, -1000])
        def p(a, b=0, c=0):
            return V(x+a, b, c)
        base = m.add_face([p(0), p(0, 2), p(4, 2), p(4)])
        wall = m.add_face([p(-2), p(6), p(6, 0, 2), p(-2, 0, 2)])
        host = m.add_face([p(-4, -4), p(8, -4), p(8, 4), p(-4, 4)],
                          [[p(-2), p(6), p(6, 1), p(-2, 1)]])
        faces = [wall, host, base]
        rng.shuffle(faces)
        for group in (faces, [wall, base], [base], [host, host, wall]):
            memo = {}
            for a, b in ((p(0), p(4)), (p(1), p(2)), (p(7, 2), p(7, 3)),
                         (p(-2, .000049), p(6, -.000049)), (p(4), p(0))):
                kind, f = topology.classify_push_edge(base, a, b, iter(group), memo)
                rows.append((kind, None if f is None else faces.index(f)))
    return rows


def plane_rows():
    rows = []
    for seed in range(36):
        rng = random.Random(seed)
        scene = Scene()
        m = scene.mesh
        x = rng.choice([0, 650000])
        z = rng.choice([-3, 0, 100])
        def p(i, j, lift=0):
            return V(x+i, j, z+lift+i*(seed%3))
        for i in range(seed%5):
            m.add_face([p(i, 0), p(i+1, 0), p(i+1, 1), p(i, 1)])
        if seed%2:
            m.add_edge(p(0, 2), p(2, 2)).curve = 5
        if seed%4 == 1:
            m.vertex(p(1, 3, .2))
        pts = [p(1, 0), p(2, 1), p(0, 1, rng.choice([0, .0001, .3]))][:seed%4]
        plane = edits._whole_drawing_plane(scene, pts)
        commands = []
        edits._append_flat_curve_rebuild(scene, commands, pts, close_cycles=seed%3 != 1)
        edits._append_face_plane_rebuild(scene, commands, pts)
        rows.append((None if plane is None else [xyz(v) for v in plane],
                     [(type(c).__name__, xyz(c.origin) if hasattr(c, 'origin') else None,
                       xyz(c.normal) if hasattr(c, 'normal') else None) for c in commands]))
    return rows


def edge_rows():
    rows = []
    for seed in range(24):
        rng = random.Random(seed)
        m = Mesh()
        verts = [m.vertex(V(i, seed, 0)) for i in range(8)]
        start = m._mut_serial
        keys = {}
        events = []
        for j in range(48):
            a, b = rng.sample(verts, 2)
            e = m.add_edge(a.position, b.position)
            events.append((keys.setdefault(e.key, len(keys)), e is m.find_edge(b, a),
                           verts.index(e.v0), verts.index(e.v1)))
            if j%5==0:
                m.remove_edge(e)
        snap = m.capture_state()
        old = list(m.edges)
        m.remove_edge(old[-1])
        m.restore_state(snap)
        rows.append((events, m.edges == old, m._mut_serial-start,
                     [[keys[e.key] for e in m.edges if e in v.edges] for v in verts]))
    return rows


def snap_rows():
    rows = []
    for seed in range(40):
        rng = random.Random(seed)
        trace = []
        def hidden(p):
            trace.append(('one', xyz(p)))
            return abs(p.x()) < seed%5
        def many(ps):
            trace.append(('many', [xyz(p) for p in ps]))
            return [abs(p.x()) < seed%5 for p in ps]
        points = [V(i%7, 0, 0) for i in range(34)]
        hide = hidden if seed%4 else None
        batch = many if seed%3 else None
        index = snap._first_visible(points, hide, batch)
        candidates = [(rng.choice([.1, .4, .6, 1.]), p, i, None, i if i%3 else None, i%4!=1)
                      for i, p in enumerate(points)]
        hit = snap._resolve_point_candidates(candidates, hide, batch)
        ref, aim = V(-2, 1, 0), V(seed%6, seed%7-3, 1)
        def proj(p):
            return None if p.z() > 6 else (p.x(), p.y())
        def foot(ref, axis):
            return ref + axis*V.dotProduct(aim-ref, axis)
        args = (ref, aim, aim.x()+.2, aim.y(), proj, 3, hide)
        s = snap._first_point_from_point(*args, V(1, 0, 0), foot, seed%2==0)
        t = snap._in_plane_with_point_snap(ref, aim, V(1, 0, 0), *args[2:])
        def guide(h):
            if h is None:
                return None
            return [xyz(h.point), h.kind, h.axis, [xyz(p) for p in h.guide],
                    [(xyz(a), xyz(b), c) for a, b, c in h.guides or ()], h.guide_color]
        rows.append((index, hit[2], guide(s), guide(t), trace))
    return rows


def test_materialised_classification_reference(monkeypatch):
    monkeypatch.setenv('INGETRAZO_STORE', '0')
    assert digest(push_rows()) == '1ec1b7e5f0d99bd68c917cd9000f24a09700f2f99f30ec09cc0f83130fbaf3db'


def test_unscanned_plane_reference(monkeypatch):
    from core import scan_index
    monkeypatch.setenv('INGETRAZO_STORE', '0')
    monkeypatch.setattr(scan_index, 'scan', lambda mesh: None)
    assert digest(plane_rows()) == '62103ffd8bfc2113d099e27a5b34c4b8a7bd5d480d1cfcefe31a117aebe77d76'


def test_scalar_edge_reference(monkeypatch):
    monkeypatch.setenv('INGETRAZO_STORE', '0')
    assert digest(edge_rows()) == '71481f4dd364be8df06114141574acfab49e2ef191d25f8351aa3adf9a341457'


def test_guides_and_visibility_reference():
    assert digest(snap_rows()) == '6f84d059578ff4161efd18b9599853470e1e6256d48b5386b6ae56800787df1c'
