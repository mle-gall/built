"""Time store kernel entry points on generated geometry, in milliseconds."""
import json
from statistics import median
from time import perf_counter
from types import SimpleNamespace

from built_core import Vec3 as V
from built_core import arrangement, push_scans, snap_query, topology
from built_core.mesh import Mesh


def cases():
    mesh = Mesh()
    for i in range(400):
        x,y = i%20,i//20
        mesh.add_face([V(x,y,0),V(x+1,y,0),V(x+1,y+1,0),V(x,y+1,0)])
    refs = [SimpleNamespace(a=e.a, b=e.b, soft=False, hidden=False, curve=None,
                            figure=False, center=False, component_origin=False,
                            arc_midpoint=False, guide=False, context=None)
            for e in mesh.edges]*4
    xy = [v.position.toTuple() for v in mesh.vertices]
    ring = [(float(i), float(i%7)) for i in range(800)]
    return {
        'query_edges': lambda: snap_query.make_query(refs, lambda p:p.toTuple()[:2], (0.,0.)),
        'landing': lambda: topology.defects_near(mesh.vertices[:30]),
        'bounds': lambda: push_scans._grown_box(xy),
        'plane_metrics': lambda: (arrangement._area_floor(ring),arrangement._signed_area(ring)),
        'rings': lambda: [f.vertices for f in mesh.faces],
    }


if __name__ == '__main__':
    result = {}
    for name,run in cases().items():
        run()
        times = []
        for _ in range(11):
            start = perf_counter()
            run()
            times.append(1000*(perf_counter()-start))
        result[name] = median(times)
    print(json.dumps(result))
