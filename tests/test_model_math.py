"""Binary64 model arithmetic before storage, after copying and at the API's edges."""
import copy
import pickle

import pytest
from ingetrazo_core import Matrix4, Quaternion, Vec3


def test_survey_coordinates_keep_millimetres_across_matrix_operations():
    point = Vec3(650000.001, 6860000.002, 100.003)
    m = Matrix4()
    m.translate(point)
    m.rotate(90, 0, 0, 1)
    local = Vec3(.003, .002, .001)
    mapped = m.map(local)
    assert mapped.toTuple() == pytest.approx((649999.999, 6860000.005, 100.004), abs=2e-9, rel=0)
    inv, valid = m.inverted()
    assert valid
    assert inv.map(mapped).toTuple() == pytest.approx(local.toTuple(), abs=2e-9)
    assert copy.deepcopy(point) == point
    assert pickle.loads(pickle.dumps(m)).data() == m.data()


def test_hashed_coordinates_stay_valid_dict_keys():
    point = Vec3(1, 2, 0)
    point.setZ(3)
    lookup = {point: True}
    assert lookup[Vec3(1, 2, 3)]
    with pytest.raises(TypeError):
        point.setX(0)
    with pytest.raises(TypeError):
        point += Vec3(1, 0, 0)
    clone = copy.copy(point)
    clone.setX(4)
    assert lookup[point] and clone != point

    # Python hashes NaNs by object identity. Reboxing the coordinates for
    # each lookup would give this same vector different hashes.
    diagnostic = Vec3(float("nan"), 0, 0)
    lookup[diagnostic] = True
    allocated = []
    for i in range(10):
        allocated.append(float(i))
        assert lookup[diagnostic]


def test_boundary_constructors_and_axis_angle():
    source = Matrix4()
    source.translate(1, 2, 3)
    v = (1.0, 0.0, 0.0)                  # a plain triple is a point too
    assert Matrix4(source).map(v) == Vec3(2, 2, 3)
    rotation = Quaternion.fromAxisAndAngle(Vec3(0, 0, 1), 90)
    assert rotation.rotatedVector(v).toTuple() == pytest.approx((0, 1, 0))
