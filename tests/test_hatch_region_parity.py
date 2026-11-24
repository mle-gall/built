"""Ordered hatch segments and DXF quads recorded from the Python kernel."""
import hashlib
import random
import struct

from core.hatches import clip_segments, fill_quads


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
