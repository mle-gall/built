"""Keyed moves and affine maps preserve folds, UVs and transaction boundaries."""
import pytest
from built_core import Matrix4, MeshStore, Vec3
from built_core import store_mesh
from core.history import History, MoveVerticesCommand, SnapshotMutation
from built_core import group_remap_uvws as _remap_uvws
from core.mesh import Mesh
from core.scene import Scene
from core.topology import _key
from tests.test_store_mesh import fingerprint


def quad():
    scene = Scene()
    mesh = scene.mesh
    face = mesh.add_face([(0.,0.,0.),(2.,0.,0.),(2.,2.,0.),(0.,2.,0.)])
    face.attrs.update({"texture":{"sw":2., "sh":3.}, "label":"keep"})
    return scene


@pytest.mark.parametrize("partial,carry", [(True,False),(True,True),(False,True),(False,False)])
def test_command_move_matches_fold_and_texture_results(monkeypatch, partial, carry):
    a,b = quad(),quad()
    mesh,ref = a.mesh,b.mesh
    original = mesh.faces[0]
    original_key = original.key
    held_attrs = original.attrs
    positions = original.vertices[:1] if partial else original.vertices
    keys = [mesh.vertex_at(p).key for p in positions]
    delta = Vec3(0.,0.,1.)
    serial = mesh._mut_serial
    moved, landed, folded, defects, dirty = MeshStore.command_move_vertices(
        mesh, [p.toTuple() for p in positions], delta.toTuple(), carry)
    command = MoveVerticesCommand(positions,delta,carry_textures=carry)
    command.do(b)
    assert moved == keys and landed == [] and defects == []
    assert folded == ([original_key] if partial else [])
    assert mesh._mut_serial - serial == dirty
    assert mesh._mut_serial == ref._mut_serial and mesh.revision == ref.revision
    assert fingerprint(mesh) == fingerprint(ref)
    if carry and not partial:
        assert held_attrs is original.attrs and "uvw" in held_attrs["texture"]


@pytest.mark.parametrize("carry", [False,True])
def test_command_transform_matches_affine_map(monkeypatch,carry):
    a,b = quad(),quad()
    mesh = a.mesh
    positions = mesh.faces[0].vertices
    matrix = Matrix4()
    matrix.translate(12.,3.,1.)
    matrix.rotate(29.,0.,1.,0.)
    matrix.scale(-2.,1.5,1.)
    keys = [v.key for v in mesh.vertices]
    moved,landed,folded,defects,dirty = MeshStore.command_transform_vertices(
        mesh,[p.toTuple() for p in positions],matrix.data(),carry)
    for vertex in b.mesh.vertices:
        b.mesh.move_vertex(vertex, matrix.map(vertex.position) - vertex.position)
    if carry:
        _remap_uvws(b.mesh, matrix, list(b.mesh.textured_faces()))
    assert (moved,landed,folded,defects,dirty) == (keys,[],[],[],len(keys))
    assert fingerprint(mesh) == fingerprint(b.mesh)


def test_defective_landing_changes_neither_store_nor_owned_paint(monkeypatch):
    from tests.test_move_curved_edge_down_1294 import _curved_front_box, H,L,W
    scene,_,_ = _curved_front_box(1,0.)
    mesh = scene.mesh
    top=[(x,y,H) for x,y in ((0.,0.),(L,0.),(L,W),(0.,W))]
    for f in mesh.faces:
        f.attrs["texture"]={"sw":1.2,"sh":0.7}
    before = fingerprint(mesh), mesh._store.revision, mesh.revision
    moved,landed,folded,defects,dirty=MeshStore.command_move_vertices(mesh,top,(0.,0.,-H),True)
    assert defects and defects[0] in ("duplicate_face","overlapping_face","pinched_face","flat_face","zero_edge","crossed_face")
    assert (moved,landed,folded,dirty)==([],[],[],0)
    assert (fingerprint(mesh),mesh._store.revision,mesh.revision)==before


def test_receiver_keys_and_occurrence_replay(monkeypatch):
    scene=Scene()
    mesh=scene.mesh
    receiver=mesh.add_edge((2.,0.,0.),(3.,0.,0.)).v0.key
    source=mesh.add_edge((0.,0.,0.),(0.,1.,0.)).v0.key
    matrix=Matrix4()
    matrix.translate(10.,0.,0.)
    occ=store_mesh.instance_view(mesh,matrix.data())
    out=[]
    history=History(scene)
    history.execute(SnapshotMutation(lambda _: out.append(MeshStore.command_move_vertices(
        occ,[(10.,0.,0.)],(2.,0.,0.))),mesh=occ))
    assert out[0][:4]==([source],[receiver],[],[])
    after=fingerprint(mesh)
    assert history.undo() and mesh._store.is_live(source)
    assert history.redo() and fingerprint(mesh)==after


def test_vertex_refusal_precedes_writes(monkeypatch):
    mesh=quad().mesh
    before=fingerprint(mesh),mesh.revision
    assert MeshStore.command_move_vertices(mesh,[(1e12,0.,0.)],(10.,0.,0.)) == ([],[],[],[],0)
    assert (fingerprint(mesh),mesh.revision)==before
