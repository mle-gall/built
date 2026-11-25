"""The ordered geometry and drawing returned by the public HLR entry points."""
import gzip
import json
from pathlib import Path

import numpy as np

from core import hlr
from built_core.section import SectionPlane
from built_core import Vec3
from tests.test_export_view_svg import _camera
from tests.test_hlr_tagged_geometry import _scene

FIXTURE = Path(__file__).parent / "fixtures" / "hlr_walk_python.json.gz"


def snapshot():
    scene = _scene(None, False)
    camera = _camera(scene)
    result = {"geometry": hlr.collect_geometry(scene, layers=True)}
    for section in (False, True):
        if section:
            plane = SectionPlane(Vec3(0, 0, 1), Vec3(0, 0, 1))
            scene.section_planes.append(plane)
            scene.set_active_section(plane)
        camera.perspective = False
        drawing = hlr.hlr_drawing(scene, camera, layers=True, hidden=True,
                                  depth_layers=["Muros"])
        result[f"parallel-{section}"] = {
            "segments": drawing.segs.tolist(), "world": drawing.world.tolist(),
            "kinds": drawing.kinds.tolist(), "layers": drawing.layers.tolist(),
            "loops": [r.tolist() for r in drawing.loops],
            "loop_layers": drawing.loop_layers, "loop_depths": drawing.loop_depths,
        }
        camera.perspective = True
        result[f"perspective-{section}"] = hlr.hlr_perspective(scene, camera).tolist()
    return result


def test_ordered_hlr_walk_matches_python():
    def compare(a, b):
        if isinstance(b, dict):
            assert a.keys() == b.keys()
            for key in b:
                compare(a[key], b[key])
        elif isinstance(b, list):
            assert len(a) == len(b)
            for left, right in zip(a, b):
                compare(left, right)
        elif isinstance(b, float):
            np.testing.assert_allclose(a, b, atol=1e-12, rtol=0)
        else:
            assert a == b

    compare(snapshot(), json.loads(gzip.decompress(FIXTURE.read_bytes())))


if __name__ == "__main__":
    import sys
    if sys.argv[1:] == ["--write"]:
        FIXTURE.write_bytes(gzip.compress(json.dumps(snapshot()).encode(), mtime=0))
