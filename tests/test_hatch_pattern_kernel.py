"""Drawing mark order and geometry recorded from the original hatch engine."""
import hashlib
import json
import random

from core import hatches


def test_generated_patterns_match_recorded_paper_geometry():
    rng = random.Random(87351)
    digest = hashlib.sha256()

    def coordinates(item):
        if isinstance(item, dict):
            return {key: coordinates(value) for key, value in item.items()}
        if isinstance(item, (tuple, list)):
            return [coordinates(value) for value in item]
        return int(round(item * 1e7))

    for key in hatches.HATCH_ORDER:
        for angle in (0., 23., 190.):
            box = (-3.5, -2.1, rng.uniform(3., 10.), rng.uniform(3., 12.))
            marks = hatches.hatch_marks(key, box, rng.uniform(.8, 2.), angle,
                                         offset=(.3, -.7))
            digest.update(json.dumps(coordinates(marks), sort_keys=True).encode())
    assert digest.hexdigest() == "608b03f42d3a59256179227090be85d11bbeda7f5d9f3ffaa3009e54d9250ac8"
