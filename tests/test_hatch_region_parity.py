"""Ordered hatch segments and DXF quads recorded from the Python kernel."""
import hashlib
import random
import struct

from built_core.hatches import clip_segments, fill_quads


def test_seeded_paper_regions():
    rng = random.Random(78321)
    digest = hashlib.sha256()
    for _ in range(240):
        rings = [[(rng.randint(-20, 20), rng.randint(-20, 20))
                  for _ in range(rng.randrange(12))]
                 for _ in range(rng.randrange(1, 5))]
        lines = [tuple(rng.randint(-30, 30) for _ in range(4)) for _ in range(20)]
        for row in clip_segments(lines, rings):
            digest.update(struct.pack(">4d", *row))
        digest.update(b"|")
        for quad in fill_quads(rings):
            for point in quad:
                digest.update(struct.pack(">2d", *point))
        digest.update(b"|")
    assert digest.hexdigest() == "eb42470fb2a1f889d22280a7d532579536d96ab546ba4cd1ba18f71117348503"


def test_catalogue_on_seeded_section_bands():
    """Clipped DXF segments, sheet marks and PAT families from the Python catalogue."""
    import json
    from built_core import hatches

    def rounded(value):
        if isinstance(value, float):
            return round(value, 10) or 0
        if isinstance(value, (list, tuple)):
            return [rounded(v) for v in value]
        if isinstance(value, dict):
            return {k: rounded(v) for k, v in value.items()}
        return value

    rng = random.Random(1706)
    catalogue = hashlib.sha256()
    for key in hatches.HATCH_ORDER:
        cases = []
        for cut in ("plan", "vertical", None):
            x, y = (rng.uniform(-15, 0) for _ in range(2))
            w, d = (rng.uniform(4, 15) for _ in range(2))
            ring = [(x, y), (x+w, y), (x+w, y+d), (x, y+d)]
            form = dict(cut=cut, scale_n=25, scale=.85, angle=17,
                        cross=True, offset=(2, -3))
            cases.extend((hatches.hatch_lines(key, [ring], form),
                          hatches.hatch_marks(key, (x, y, x+w, y+d), .85, 17),
                          hatches.line_families(key)))
        record = json.dumps(rounded(cases), sort_keys=True).encode()
        catalogue.update(f"{key} {hashlib.sha256(record).hexdigest()}\n".encode())
    assert catalogue.hexdigest() == "cf1875fc29416c0f9a1794ebf41acdaa09e5f04ba46bab381771bc5452155c49"


def test_script_rings_accept_json_coordinates():
    from built_core import hatches

    points = [(0, 0), (8, 0), (8, 4), (0, 4)]
    decoded = [[str(x), str(y)] for x, y in points]
    assert hatches.vertical_band(decoded) == hatches.vertical_band(points)
    assert hatches.hatch_lines("grid", [decoded]) == hatches.hatch_lines("grid", [points])
    assert hatches.shift_marks({"dots": [[1, 2]], "lines": [[0, 1, 2, 3]]}, 2, 4) == {
        "lines": [(2, 5, 4, 7)], "dots": [(3, 6)], "circles": []}
