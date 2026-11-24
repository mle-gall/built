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


def test_masonry_courses_and_cut_marks_match_paper_geometry():
    """Include wall plans, cross sections, and a thick piece showing the bond face."""
    rng = random.Random(5924)
    digest = hashlib.sha256()

    def record(value):
        if isinstance(value, (tuple, list)):
            return [record(v) for v in value]
        if isinstance(value, dict):
            return {k: record(v) for k, v in value.items()}
        if isinstance(value, float):
            return round(value * 1e7)
        return value

    for key in sorted(hatches.COURSED):
        for thickness in (1.1, 3.1, 35.5):
            for cut in ("plan", "vertical"):
                for joint in ("single", "double"):
                    band = ((-1.7, 9.8), (-.6, -.8), (.8, -.6), 15.2, thickness)
                    offset = (rng.uniform(-250, 250), rng.uniform(-250, 250))
                    brick = (240., 100., 67., 8., 15.)
                    course = hatches.bond_course(key, -3, thickness * 50., brick, -313., 631.)
                    marks = hatches.bond_marks(key, band, cut, 1.5, 50., brick, joint, offset)
                    digest.update(json.dumps(record((course, marks)), sort_keys=True).encode())
    assert digest.hexdigest() == "de3127269e7e033c819f41c95050380b5c6f0a375ccdee009b7067dbdcbc8272"
