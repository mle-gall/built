"""Recorded R1.3 pointer answers, including placement and filtered edge identity."""
from PySide6.QtGui import QMatrix4x4, QVector3D as V
import pytest

from core.group import Group
from core.mesh import Mesh
from core.scene import Scene
from tests.test_metal_parity_inference import at, harness
from tests.test_.skp_inference_gaps import view  # noqa: F401


def placed_scene(placement):
    mesh = Mesh()
    mesh.add_edge(V(1, 1, 0), V(3, 1, 0))
    if placement == 'loose':
        scene = Scene()
        scene.mesh.add_edge(V(1, 1, 0), V(3, 1, 0))
        return scene
    leaf = Group(mesh)
    parent = Group(Mesh())
    parent.adopt([leaf])
    if placement == 'instance':
        leaf.xform = QMatrix4x4()
        leaf.component = True
        leaf.xform.translate(0, 1, 0)
    return Scene(groups=[parent])


@pytest.mark.parametrize('placement,y,context', [
    ('loose', 1, None), ('nested', 1, 'group'), ('instance', 2, 'component')])
@pytest.mark.parametrize('cursor,kind,x,edge_id', [
    (1.02, 'endpoint', 1, None), (2.02, 'midpoint', 2, None),
    (2.41, 'on_edge', 2.41, 0)])
def test_recorded_geometry(placement, y, context, cursor, kind, x, edge_id):
    view, index = harness(placed_scene(placement))
    result = at(view, index, (cursor, y+.01, 0))
    assert (result.kind, result.axis, result.context) == (kind, None, context)
    assert result.point.toTuple() == pytest.approx((x, y, 0), abs=1e-6)
    assert getattr(result.edge, 'edge_id', None) == edge_id
    if result.edge is not None:
        assert view.packed.groups[result.edge.row] is result.edge.group


@pytest.mark.parametrize('mode', ['axis', 'parallel', 'perpendicular', 'screen', 'two-point'])
def test_recorded_modes(mode):
    from types import SimpleNamespace
    view, index = harness(placed_scene('loose'))
    p = V(3, 3, 0)
    options = dict(start_point=V(1, 3, 0),
                   project_onto_line=lambda s,d: s+d*(V.dotProduct(p-s,d)/d.lengthSquared()))
    kind, axis = 'reference', None
    if mode == 'axis':
        options.update(axis_lock='x')
        kind, axis = 'axis', 'x'
    elif mode == 'screen':
        options.update(screen_axis_px=3.)
        kind, axis = 'axis', 'x'
    elif mode == 'two-point':
        options.update(start_point=None, acquired_points=[V(3,2,0), V(2,3,0)])
        kind = 'from_point'
    else:
        end = V(1,0,0) if mode == 'parallel' else V(0,1,0)
        options.update(reference_edge=SimpleNamespace(a=V(), b=end), reference_mode=mode)
    result = at(view, index, p.toTuple(), **options)
    assert (result.kind, result.axis, result.edge) == (kind, axis, None)
    assert result.point.toTuple() == pytest.approx(p.toTuple(), abs=1e-6)


def test_recorded_lambert(view):
    from tests.test_.skp_inference_gaps import refresh, move
    origin = (650000., 6860000.)
    square = [(.01,.02), (.03,.02), (.03,.04), (.01,.04)]
    view.scene.mesh.add_face([(origin[0]+x, origin[1]+y, 0.) for x,y in square])
    refresh(view)
    view.camera.target = V(.02,.03,0)
    view.camera.distance = .12
    for x,y in square:
        result = move(view, V(x,y,0))
        assert (result.kind, result.axis, result.edge) == ('endpoint', None, None)
        assert result.point.toTuple() == pytest.approx((x,y,0), abs=1e-8)
