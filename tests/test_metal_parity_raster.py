"""wgpu frame checks on a headless Core Animation layer
(tests/headless_gpu.py): the release renderer, no window and no toolkit."""
import importlib.util
import os
import numpy as np
import pytest
from core.scene import Scene
from views.wgpu_style import style_payload, sun_payload

HAS_RENDERER = importlib.util.find_spec("ingetrazo_render") is not None


def raster(rgb):
    """A ``SurveyRaster`` of an H×W×3 (opaque) or H×W×4 array."""
    from ingetrazo_render import SurveyRaster
    rgb = np.asarray(rgb, np.uint8)
    if rgb.shape[2] == 3:
        rgb = np.concatenate([rgb, np.full(rgb.shape[:2] + (1,), 255, np.uint8)], axis=2)
    return SurveyRaster(rgb.shape[1], rgb.shape[0], np.ascontiguousarray(rgb).tobytes())


def filled(w, h, colour):
    return raster(np.broadcast_to(np.array(colour, np.uint8), (h, w, len(colour))))


def test_payload_defaults_and_hidden_controls():
    scene = Scene()
    p = style_payload(scene)
    assert p['material_opacity'] is True
    assert p['shadows'] is False
    assert p['group_selection_edges'] is True
    assert not p['show_hidden_geometry'] and not p['show_hidden_objects']
    scene.show_hidden_geometry = scene.show_hidden_objects = True
    scene.wgpu_material_opacity = True
    p = style_payload(scene)
    assert p['show_hidden_geometry'] and p['show_hidden_objects'] and p['material_opacity']


def test_sun_matches_gl_and_can_be_disabled(monkeypatch):
    from ingetrazo_core import Vec3 as QVector3D
    from tests.controller_surface import ControllerSurface as Viewport
    scene = Scene()
    scene.loose_mesh.add_face([QVector3D(0,0,0), QVector3D(1,0,0), QVector3D(0,1,1)])
    scene.shadows.enabled = True
    scene.shadows.hour = 12
    p = sun_payload(scene)
    view = Viewport(); view.scene = scene
    assert p['shadows']
    np.testing.assert_allclose(p['sun_direction'], view._sun_dir_now())
    assert len(p['shadow_matrix']) == 16
    assert sun_payload(scene, override=scene.display_style)['shadows']
    from ingetrazo_core.style import Style
    assert not sun_payload(scene, override=Style(name='tecnico'))['shadows']
    monkeypatch.setenv('INGETRAZO_NO_SHADOWS','1')
    assert not sun_payload(scene)['shadows']
    view.close()


@pytest.mark.skipif(not HAS_RENDERER, reason='release ingetrazo_render required')
def test_native_api_available():
    from ingetrazo_render import Renderer
    for name in ('set_preview_mesh','set_face_marks','set_suppressed_faces',
                 'set_profile_exclusions','set_hidden_geometry','set_lines'):
        assert hasattr(Renderer, name)


@pytest.fixture
def native():
    if not HAS_RENDERER:
        pytest.skip('requires the render crate')
    from ingetrazo_core import Matrix4 as QMatrix4x4
    from tests.headless_gpu import renderer as headless
    renderer = headless(64, 64, 1.)
    view=QMatrix4x4(); view.translate(0,0,-3)
    proj=QMatrix4x4(); proj.ortho(-1,1,-1,1,.1,10)
    renderer.set_camera(list(view.data()),list(proj.data()))
    renderer.set_antialias(False); renderer.set_edges(False)
    yield renderer
    del renderer


def synthetic_pack(alpha=64):
    from ingetrazo_core import VERTEX, INSTANCE, COMMAND
    vertices=np.zeros(12,dtype=VERTEX)
    quad=np.array([[-.8,-.8,0],[.8,-.8,0],[.8,.8,0],[-.8,-.8,0],[.8,.8,0],[-.8,.8,0]],dtype=np.float32)
    vertices['pos'][:6]=quad; quad[:,2]=-.4; vertices['pos'][6:]=quad
    vertices['normal'][:,2]=1
    vertices['color'][:6]=[255,0,0,alpha]; vertices['color'][6:]=[0,0,255,255]
    indices=np.arange(12,dtype=np.uint32)
    bounds=np.array([[[-.8,-.8,-.4,0],[.8,.8,0,0]]],dtype=np.float32)
    commands=np.zeros(2,dtype=COMMAND); commands['count'][0]=12
    rows=np.zeros(1,dtype=INSTANCE); rows['world'][0]=np.eye(4,dtype=np.float32)
    return vertices,indices,bounds,commands,rows


def upload(renderer, alpha=64):
    v,i,b,c,r=synthetic_pack(alpha)
    renderer.set_scene(tuple(x.view(np.uint8).ravel() for x in (v,i,b,c)),r.view(np.uint8).ravel())
    return v,i,b,c,r


def capture(renderer):
    renderer.request_capture(); renderer.render(False)
    return np.frombuffer(bytes(renderer.take_capture()),np.uint8).reshape(64,64,4)


@pytest.mark.parametrize('aa',[False,True])
def test_material_alpha_blends_without_hiding_opaque_background(native,aa):
    upload(native,64); native.set_antialias(aa)
    native.set_style(dict(material_opacity=True,sky=False,edges=False,profiles=False))
    rgba=capture(native)
    mixed=rgba[32,32,:3].astype(int)
    if os.environ.get('P2_RASTER_OUTPUT'):
        from pathlib import Path
        path=Path(os.environ['P2_RASTER_OUTPUT']); path.mkdir(parents=True,exist_ok=True)
        raster(rgba).save(str(path/f'synthetic-alpha64-msaa-{aa}.png'))
    np.testing.assert_allclose(mixed, [62, 0, 185], atol=3)  # 64/255 coverage in display space
    upload(native,255)
    opaque=capture(native)[32,32,:3].astype(int)
    assert opaque[0]>180 and opaque[2]<5
    assert mixed[0]<opaque[0]
    # An opaque front face hides translucent geometry behind it.
    v,i,b,c,r=synthetic_pack(64)
    v['pos'][:6,2]=-.8
    b[0,0,2]=-.8
    native.set_scene(tuple(x.view(np.uint8).ravel() for x in (v,i,b,c)),r.view(np.uint8).ravel())
    behind=capture(native)[32,32,:3]
    assert behind[0]<5 and behind[2]>180


def test_suppression_restores_and_rejects_invalid_ranges(native):
    upload(native,255); native.set_style(dict(material_opacity=True,sky=False))
    original=capture(native)
    native.set_suppressed_faces([(0,0,2)])
    hidden=capture(native)
    assert hidden[32,32,2]>150 and hidden[32,32,0]<5
    native.set_suppressed_faces([])
    np.testing.assert_array_equal(original,capture(native))
    with pytest.raises(ValueError): native.set_suppressed_faces([(0,3,2)])
    np.testing.assert_array_equal(original,capture(native))


@pytest.mark.parametrize('aa',[False,True])
def test_preview_and_face_marks_are_depth_tested(native,aa):
    v,*_=upload(native,255); native.set_antialias(aa)
    native.set_style(dict(sky=False,group_selection_edges=True))
    original=capture(native)
    empty=np.zeros(0,np.uint8)
    preview=v[:6].copy(); preview['pos'][:,2]=-.2; preview['color'][:]=[0,255,0,255]
    native.set_preview_mesh(preview.view(np.uint8).ravel(),empty)
    np.testing.assert_array_equal(original,capture(native))
    preview['pos'][:,2]=.2
    native.set_preview_mesh(preview.view(np.uint8).ravel(),empty)
    assert capture(native)[32,32,1]>150
    native.set_preview_mesh(empty,empty)
    np.testing.assert_array_equal(original,capture(native))
    marks=v[:6].copy(); marks['color'][:]=[0,0,255,255]
    native.set_face_marks(marks.view(np.uint8).ravel())
    marked=capture(native)
    assert np.count_nonzero(marked[:,:,2]>original[:,:,2])>20
    # WGSL fragment coordinates start at the top, the selection mask at the bottom.
    mask = marked[20:44, 20:44, 2] > original[20:44, 20:44, 2]
    y, x = np.mgrid[20:44, 20:44]
    np.testing.assert_array_equal(mask, ((x % 6) < 2) & (((63 - y) % 6) < 2))
    native.set_face_marks(empty)
    np.testing.assert_array_equal(original,capture(native))


def test_tool_wire_draws_over_faces_and_clears(native):
    upload(native, 255)
    native.set_style(dict(sky=False, edges=False))
    original = capture(native)
    band = ([((-.7, 0., -.5), (.7, 0., -.5))], (0., 1., 0., 1.), 3.)
    native.set_tool_wire(band, (), 1.)
    marked = capture(native)
    assert (marked[31:33, 15:49, 1] > 200).all()
    native.set_tool_wire(None, (), 1.)
    np.testing.assert_array_equal(original, capture(native))


def test_loose_transform_preview_restores_resident_faces(native):
    vertices, _, _, _, _ = upload(native, 255)
    native.set_style(dict(sky=False, edges=False))
    original = capture(native)
    selected = vertices["pos"][:6].tolist()
    assert native.begin_tool_geometry(0, vertices.view(np.uint8).ravel(), selected, [0., 0., 0.])
    transform = np.eye(4)
    transform[3, 0] = 3.
    native.pose_tool_geometry(transform.ravel().tolist())
    moved = capture(native)
    assert moved[32, 32, 2] > moved[32, 32, 0]
    native.end_tool_geometry()
    np.testing.assert_array_equal(original, capture(native))


def test_texture_tool_updates_only_one_faces_uv_rows(native, tmp_path):
    from types import SimpleNamespace
    from ingetrazo_core import Vec3
    from tools.texture_position import TexturePositionTool
    from tools.texture_position_metal import TexturePreview
    from ingetrazo_core import pack_scene

    path = tmp_path / "tile.png"
    tile = np.zeros((16, 16, 3), np.uint8)
    tile[:, :8, 0] = tile[:, 8:, 2] = 255                    # red, then blue
    assert raster(tile).save(str(path))
    scene = Scene()
    for x in (-.85, .1):
        face = scene.mesh.add_face([Vec3(x+u, v, 0) for u, v in
                                    ((0, -.7), (.7, -.7), (.7, .7), (0, .7))])
        face.attrs["texture"] = {"path": str(path), "sw": 1., "sh": 1.}
    pack = pack_scene(scene)
    pack.upload(native)
    native.set_style(dict(sky=False, edges=False))
    view = SimpleNamespace(scene=scene, packed=pack, renderer=native, _instance_updates=True)
    tool = TexturePositionTool()
    assert tool.begin(view, face, face.centroid())
    preview = TexturePreview(view)
    before = capture(native)
    memory = native.gpu_memory
    tool.map.R += Vec3(.5, 0, 0)
    preview.update(tool)
    shifted = capture(native)
    np.testing.assert_array_equal(before[:, :31], shifted[:, :31])
    assert np.count_nonzero(np.any(before[:, 34:] != shifted[:, 34:], axis=2)) > 80
    assert native.gpu_memory == memory
    preview.restore()
    np.testing.assert_array_equal(before, capture(native))


def test_tool_pick_same_pixel_tracks_instance_changes(native):
    _, _, _, _, rows = upload(native, 255)
    assert native.pick(32, 32) == 0
    assert native.pick(32, 32) == 0
    rows["world"][0, 3, 0] = 4.
    native.update_instances([0], rows.view(np.uint8).ravel())
    assert native.pick(32, 32) == -1
    rows["world"][0, 3, 0] = 0.
    native.update_instances([0], rows.view(np.uint8).ravel())
    assert native.pick(32, 32) == 0


@pytest.mark.parametrize('aa',[False,True])
def test_scaffold_lines_are_depth_tested_and_dashed(native,aa):
    """The edit box and guides (``lines.rs``): hidden behind a face, drawn
    in front of it and on it (the edges' bias), dashed in screen pixels."""
    from views.wgpu_lines import segments
    upload(native,255); native.set_antialias(aa)
    native.set_style(dict(sky=False,edges=False))
    original=capture(native)
    def line(z,dash=0.,y=0.):
        data=segments([[[-.7,y,z],[.7,y,z]]],(0,255,0,255),3.,dash)
        native.set_lines('test',data.view(np.uint8).ravel())
        return capture(native)
    green=lambda rgba: rgba[:,:,1].astype(int)>150
    behind=line(-.2)                          # between the red and blue quads
    np.testing.assert_array_equal(original,behind)
    front=line(.2)
    assert green(front)[30:34,12:52].any(axis=0).all(), 'solid line spans the face'
    on_face=line(0.)
    assert green(on_face)[30:34,12:52].any(axis=0).all(), 'a line on a face wins LEQUAL'
    dashed=green(line(.2,dash=4.))[30:34,10:54].any(axis=0)
    runs=np.flatnonzero(np.diff(dashed.astype(int)))
    assert 8<len(runs), 'on/off runs along the line'
    assert np.all(np.diff(runs)>=2) and np.all(np.diff(runs)<=6)
    # Sheet captures never carry scaffolding.
    from ingetrazo_core import Matrix4 as QMatrix4x4
    view=QMatrix4x4(); view.translate(0,0,-3)
    proj=QMatrix4x4(); proj.ortho(-1,1,-1,1,.1,10)
    style=dict(sky=False,edges=False)
    sheet=bytes(native.render_image(64,64,list(view.data()),list(proj.data()),style))
    native.set_lines('test',np.zeros(0,np.uint8))
    assert sheet==bytes(native.render_image(64,64,list(view.data()),list(proj.data()),style))
    np.testing.assert_array_equal(original,capture(native))
    with pytest.raises(ValueError): native.set_lines('test',np.zeros(39,np.uint8))
    assert dict(native.line_counts())['test']==0


@pytest.mark.parametrize('aa',[False,True])
def test_scaffold_lines_under_scene_edges_selected_hidden_above(native,aa):
    """GL's order where a line and a model edge share pixels: the box and
    guides (``_draw_scaffold``) come before the scene edges, which cover
    them; the selected group's hidden edges come after them."""
    from views.wgpu_lines import segments
    v,i,b,c,r=synthetic_pack(255)
    # One hard edge: the red quad's bottom side (a lone face's border is soft).
    i=np.concatenate([i,np.array([0,1],np.uint32)]); c['first'][1]=12; c['count'][1]=2
    native.set_scene(tuple(x.view(np.uint8).ravel() for x in (v,i,b,c)),r.view(np.uint8).ravel())
    native.set_antialias(aa)
    native.set_style(dict(sky=False,edges=True,profiles=False,edge_color=[0.,0.,0.],edge_width=5.))
    original=capture(native)
    assert (original[55:60,32,:3]<20).all(), 'the hard edge is drawn'
    # On that edge (y -.8: row 57), narrower than it.
    data=segments([[[-.6,-.8,0.],[.6,-.8,0.]]],(0,255,0,255),1.).view(np.uint8).ravel()
    green=lambda rgba: (rgba[:,:,1].astype(int)>100)&(rgba[:,:,0].astype(int)<100)
    native.set_lines('test',data)
    under=capture(native)
    native.set_lines('test',data,True)
    above=capture(native)
    native.set_lines('test',np.zeros(0,np.uint8),True)
    assert not green(under).any(), 'the scene edge paints over the scaffold'
    np.testing.assert_array_equal(original,under)
    assert green(above)[54:61].any(axis=0).sum()>30, 'selected hidden edges win over the scene edges'
    np.testing.assert_array_equal(original,capture(native))


def test_shadow_toggle_and_invalid_payload(native):
    upload(native,255)
    native.set_style(dict(shadows=False))
    original=capture(native)
    with pytest.raises(ValueError): native.set_style(dict(shadows=True,sun_direction=(0,0,0)))
    native.set_style(dict(shadows=False))
    np.testing.assert_array_equal(original,capture(native))
    from ingetrazo_core import Matrix4 as QMatrix4x4
    matrix=QMatrix4x4(); matrix.ortho(-2,2,-2,2,-2,2)
    native.set_style(dict(shadows=True,sun_direction=(1,0,0.1),shadow_matrix=list(matrix.data())))
    shaded=capture(native)
    assert shaded[32,32,0]<original[32,32,0]
    native.set_style(dict(shadows=False))
    np.testing.assert_array_equal(original,capture(native))


@pytest.mark.parametrize("aa", [False, True])
def test_paste_preview_repeats_the_image_and_clears(native, aa, tmp_path):
    from ingetrazo_core import Vec3
    from ingetrazo_render import preview_materials
    from tools.paste import PasteTool
    from tests.controller_surface import ControllerSurface

    upload(native, 255)
    native.set_antialias(aa)
    native.set_style(dict(sky=False, edges=False))
    original = capture(native)
    path = str(tmp_path / "strip.png")
    assert raster([[(255, 255, 0), (0, 255, 255)]]).save(path)
    tool = PasteTool()
    points = [Vec3(x, y, 0) for x, y in [(-.8, -.8), (.8, -.8), (.8, .8), (-.8, .8)]]
    texture = dict(path=path, uvw=[1.25, 0., 0., 1., 0., 1.25, 0., 1.])
    tool._clip = {"faces": [(points, [], {"texture": texture})]}
    tool._offset = Vec3(0, 0, .2)
    controller = ControllerSurface(None)
    plain, runs = preview_materials(tool.preview_faces(), controller, 1.)
    assert not plain
    empty = np.empty(0, np.uint8)
    native.set_preview_mesh(empty, empty, [(p, shade, opacity, np.asarray(v))
                            for p, shade, opacity, v in runs])
    controller.close()
    frame = capture(native)[32]
    for x in (13, 39):
        assert frame[x, 0] > 220 and frame[x, 2] < 35
    assert frame[26, 2] > 220 and frame[26, 0] < 35
    native.set_preview_mesh(empty, empty)
    np.testing.assert_array_equal(original, capture(native))


def test_hidden_ranges_are_opt_in(native):
    v,i,b,c,r=upload(native,255)
    hidden=v[:6].copy();hidden['pos'][:,2]=.2;hidden['color'][:]=[0,255,0,255]
    commands=c[:1].copy();commands['count']=6
    edges=commands.copy();edges['count']=0
    native.set_hidden_geometry(hidden.view(np.uint8).ravel(),i[:6].view(np.uint8),commands.view(np.uint8),edges.view(np.uint8))
    native.set_style(dict(show_hidden_geometry=False,sky=False))
    original=capture(native)
    native.set_style(dict(show_hidden_geometry=True,sky=False))
    ghost=capture(native)
    assert np.count_nonzero(ghost[:,:,1]>original[:,:,1])>100
    native.set_style(dict(show_hidden_geometry=False,sky=False))
    np.testing.assert_array_equal(original,capture(native))


@pytest.mark.parametrize('aa',[False,True])
def test_partial_texture_alpha_and_billboard_cutout(native,aa):
    v,i,b,c,r=upload(native,255)
    native.set_antialias(aa)
    native.set_style(dict(material_opacity=True,sky=False,edges=False))
    table=np.zeros((25,4),np.uint32)
    table[0]=[0,1,12,0]
    for index in range(12):
        table[1+index*2]=np.array([.5,.5,.5,.5],np.float32).view(np.uint32)
        if index<6:table[2+index*2,1:3]=1
    native.set_materials(table.view(np.uint8).ravel())
    native.set_textures(1,1,2,[[255,255,255,255,255,0,0,80]])
    mixed=capture(native)[32,32,:3]
    np.testing.assert_allclose(mixed.astype(int), [78, 0, 171], atol=3)  # 80/255 texture alpha
    # Same texture on face-me geometry has a hard 0.5 threshold.
    r['info'][0,2]=8
    native.update_instances([0],r.view(np.uint8).ravel())
    cutout=capture(native)[32,32,:3]
    assert cutout[0]<5 and cutout[2]>180


def audit_handoff():
    """Exercise P2 with synthetic P3 alpha/soft handoffs, without changing pack code.

    Usage: python tests/test_metal_parity_raster.py MODEL --capture-only
           --live-frame --audit-case CASE --output DIR
    Remaining arguments are forwarded unchanged to the established audit harness.
    """
    from pathlib import Path
    import runpy
    import sys
    import views.wgpu_view as module
    from ingetrazo_core import PackedScene
    original_pack = module.pack_scene
    original_upload = PackedScene.upload
    import views.wgpu_style as style_module
    original_style = style_module.style_payload

    def packed(scene, *args, **kwargs):
        result = original_pack(scene, *args, **kwargs)
        # Only these fixture prototypes receive synthetic effective opacity.
        seen = set()
        for row, group in enumerate(result.groups):
            name = getattr(group, 'name', '')
            alpha = {'Mirrored glass': 77, 'Translucent tag': 64}.get(name)
            if alpha is None:
                continue
            prototype = int(result.instances['info'][row,0])
            if prototype in seen:
                continue
            seen.add(prototype)
            cmd = result.commands[prototype]
            indices = result.indices[int(cmd['first']):int(cmd['first']+cmd['count'])]+int(cmd['base'])
            result.vertices['color'][indices,3] = alpha
        return result

    def upload_with_flags(self, renderer):
        original_upload(self, renderer)
        # This smooth fixture has a single loose prototype. P3's real handoff
        # should provide endpoint pairs while packing, without face realization.
        if 'smooth' in Path(sys.argv[1]).stem:
            from formats.igz import load_into
            source = Scene(); load_into(source, Path(sys.argv[1]))
            pairs = [value for edge in source.loose_mesh.edges if edge.soft
                     for point in (edge.a,edge.b) for value in point.toTuple()]
            renderer.set_profile_exclusions([(0,pairs)])

    def payload(scene, *args, **kwargs):
        p = original_style(scene, *args, **kwargs)
        p['material_opacity'] = True
        return p

    module.pack_scene = packed
    style_module.style_payload = payload
    PackedScene.upload = upload_with_flags
    runpy.run_path(str(Path(__file__).resolve().parents[1]/'scripts/wgpu_look_compare.py'),run_name='__main__')


if __name__ == '__main__':
    audit_handoff()


def test_pbr_shading_changes_pixels_and_keeps_picks(native, tmp_path):
    from ingetrazo_core import Vec3 as QVector3D
    from ingetrazo_core import pack_scene
    scene = Scene()
    face = scene.loose_mesh.add_face([QVector3D(x, y, 0) for x, y in
                                     ((-.8, -.8), (.8, -.8), (.8, .8), (-.8, .8))])
    face.attrs['color'] = (.7, .3, .1)
    native.set_style(dict(sky=False, edges=False, profiles=False))
    pack_scene(scene).upload(native)
    before = capture(native)
    points = [(32, 32), (20, 40), (1, 1)]
    picks = [native.pick(x, y) for x, y in points]
    assert picks == [0, 0, -1]
    path = tmp_path / 'normal.png'
    assert filled(2, 2, (200, 110, 236)).save(str(path))
    face.attrs['pbr'] = {'metallic': .75, 'normal_map': {'path': str(path)}}
    pack_scene(scene).upload(native)
    after = capture(native)
    assert not np.array_equal(before[32, 32], after[32, 32])
    assert [native.pick(x, y) for x, y in points] == picks
    # Replacing this prototype's records grows the table, then reuses it.
    # Both paths must keep the recipe/normal-map bindings and the pixels.
    packed = pack_scene(scene)
    offset, count = map(int, packed.materials[:16].view('<u4')[1:3])
    records = packed.materials[offset * 16:offset * 16 + count * 32]
    assert len(records) > 0
    sizes = [native.gpu_memory]
    for _ in range(8):
        native.update_prototype_materials(0, records)
        sizes.append(native.gpu_memory)
        np.testing.assert_array_equal(capture(native), after)
    assert sizes[-1] > sizes[0]
    assert any(a == b for a, b in zip(sizes, sizes[1:]))

def test_hidden_object_readback_survives_gpu_growth(native):
    """GPU-owned faces must still supply identical ghosts after an append."""
    v,i,b,c,r=synthetic_pack(255)
    r['info'][:,2]=1
    native.set_scene(tuple(x.view(np.uint8).ravel() for x in (v,i,b,c)),r.view(np.uint8).ravel())
    empty=c[:1].copy(); empty['count']=0
    def ghosts():
        native.set_hidden_geometry(v.view(np.uint8).ravel(),i.view(np.uint8),
                                   empty.view(np.uint8),empty.view(np.uint8),[0])
    native.set_style(dict(show_hidden_objects=True,sky=False))
    ghosts()
    before=capture(native)
    native.set_style(dict(show_hidden_objects=False,sky=False))
    assert np.any(before != capture(native))
    native.replace_prototype(0,v.view(np.uint8).ravel(),i.view(np.uint8),b.view(np.uint8).ravel(),12)
    ghosts()
    native.set_style(dict(show_hidden_objects=True,sky=False))
    np.testing.assert_array_equal(before,capture(native))


def test_intersected_wall_occludes_loose_slab_edges(native, monkeypatch):
    import math
    from ingetrazo_core import Vec3 as QVector3D
    from core.camera import OrbitCamera
    from core.history import History
    from core.intersect import WITH_CONTEXT
    from core.protocol.commands import run
    from ingetrazo_core.store_mesh import StoreMesh
    from tests.test_intersect_faces import _box, RED, BLUE
    from ingetrazo_core import pack_scene

    scene = Scene()
    mesh = scene.mesh
    assert isinstance(mesh, StoreMesh)
    for box in (_box(0, 0, 0, 2, 2, 2, RED),
                _box(-.5, .5, .5, 2.5, 1.5, 1.5, BLUE)):
        for face in box.mesh.faces:
            mesh.add_face(face.vertices).attrs = dict(face.attrs)
    history = History(scene)

    def slab_faces():
        return [f.key for f in mesh.faces if f.attrs["color"] == list(BLUE)]

    assert run(history, "intersect_faces", mode=WITH_CONTEXT, faces=slab_faces())["ok"]
    assert history.undo() and history.redo()
    assert run(history, "erase", faces=slab_faces())["ok"]

    camera = OrbitCamera()
    camera.target = QVector3D(2, 1, 1)
    camera.distance = 5
    camera.yaw = math.radians(-20)
    camera.pitch = math.radians(20)
    camera.aspect = 1
    native.set_camera(list(camera.view_matrix().data()), list(camera.projection_matrix().data()))
    native.set_style(dict(edges=True, profiles=False, sky=False))

    def frame():
        pack_scene(scene).upload(native)
        return capture(native)

    after_erase = frame()
    inside = [e for e in mesh.edges if not e.faces and {e.a.x(), e.b.x()} == {0, 2}]
    assert len(inside) == 4
    # Opening just the inner face exposes the same four edges. This also
    # catches an empty triangle stream for a face that can still be picked.
    inner = next(f for f in mesh.faces if not f.holes and f.net_area() == 1
                 and all(p.x() == 2 for p in f.vertices))
    assert run(history, "erase", faces=[inner.key])["ok"]
    open_with_edges = frame()
    assert run(history, "erase", edges=[e.key for e in inside])["ok"]
    assert np.any(frame() != open_with_edges)
    assert history.undo() and history.undo()

    for edge in inside:
        mesh.remove_edge(edge)
    np.testing.assert_array_equal(frame(), after_erase)

    # The near end of the slab extends half a unit beyond the wall.
    outside = [e for e in mesh.edges if not e.faces and max(e.a.x(), e.b.x()) == 2.5]
    assert len(outside) == 8
    for edge in outside:
        mesh.remove_edge(edge)
    assert np.any(frame() != after_erase)


@pytest.mark.parametrize("antialias", [False, True])
def test_transparent_export_preserves_the_interactive_background(native, antialias):
    from ingetrazo_core import Matrix4

    upload(native, alpha=255)
    native.set_antialias(antialias)
    view = Matrix4()
    view.translate(0, 0, -3)
    projection = Matrix4()
    projection.ortho(-1, 1, -1, 1, .1, 10)
    style = dict(sky=True, edges=False)
    native.set_style(style)
    live = capture(native)
    image = native.render_image(64, 64, view.data(), projection.data(),
                                dict(style, transparent_background=True))
    rgba = np.frombuffer(bytes(image), np.uint8).reshape(64, 64, 4)
    assert rgba[0, 0, 3] == 0
    assert rgba[32, 32, 3] == 255
    np.testing.assert_array_equal(capture(native), live)


def test_export_shadow_map_resolves_the_ground_shadow_edge(native):
    from ingetrazo_core import Matrix4

    vertices, indices, bounds, commands, rows = synthetic_pack(alpha=255)
    vertices["pos"][:, :2] *= .45
    vertices["pos"][:, 2] += .75
    bounds[0, 0, :3] = [-.36, -.36, .35]
    bounds[0, 1, :3] = [.36, .36, .75]
    native.set_scene(tuple(x.view(np.uint8).ravel() for x in
                           (vertices, indices, bounds, commands)), rows.view(np.uint8).ravel())
    view = Matrix4()
    view.translate(0, 0, -3)
    projection = Matrix4()
    projection.ortho(-1, 1, -1, 1, .1, 10)
    light = Matrix4()
    light.ortho(-64, 64, -64, 64, -4, 4)
    light.rotate(30, 0, 1, 0)
    style = dict(shadows=True, shadow_matrix=light.data(), sun_direction=(-.5,0,.866),
                 ground_shadows=True, shadow_darkness=.15, edges=False, sky=False,
                 background=(1.,1.,1.))
    def frame(sharp):
        pixels = native.render_image(256, 256, view.data(), projection.data(), style,
                                     sharp_shadows=sharp)
        return np.frombuffer(bytes(pixels), np.uint8).reshape(256, 256, 4)

    normal = frame(False)
    sharp = frame(True)
    # Count penumbra pixels on the white ground beside the elevated red quad.
    def transition(image):
        r, g, b = image[128, :, :3].T
        return np.count_nonzero((r == g) & (g == b) & (r > 65) & (r < 245))
    assert 0 < transition(sharp) < transition(normal)
    np.testing.assert_array_equal(normal, frame(False))

    native.resize(256, 256, 1.)
    native.set_camera(view.data(), projection.data())
    native.set_style(style)
    def live():
        native.request_capture()
        native.render(True)
        return np.frombuffer(bytes(native.take_capture()), np.uint8).reshape(256, 256, 4)
    moving = live()
    assert native.needs_shadow_fit
    native.settle_shadows()
    settled = live()
    assert not native.needs_shadow_fit
    assert transition(settled) < transition(moving)
    np.testing.assert_array_equal(settled, live())


@pytest.mark.parametrize('layer', ['image', 'map', 'terrain', 'photo'])
@pytest.mark.parametrize('aa', [False, True])
def test_reference_imagery_in_live_and_export_frames(native, layer, aa, tmp_path):
    from types import SimpleNamespace
    from ingetrazo_core import Vec3, Matrix4
    from ingetrazo_core.image_plane import ImagePlane
    from georef.datum import SceneDatum
    from georef.tiles import TileLayer, PRESETS
    from georef.terrain import TerrainObject
    from georef.photomesh import PhotoMesh, PhotoMaterial
    from views.wgpu_references import sync

    v, i, b, c, rows = synthetic_pack()
    rows['info'][:, 2] = 1
    native.set_scene(tuple(a.view(np.uint8).ravel() for a in (v, i, b, c)),
                     rows.view(np.uint8).ravel())
    style = dict(sky=False, edges=False, profiles=False)
    native.set_style(style)
    native.set_antialias(aa)
    baseline = capture(native)
    scene = Scene()
    adapter = SimpleNamespace(scene=scene, renderer=native, source_viewport=None,
                              packed=SimpleNamespace(origin=(0., 0., 0.)))
    adapter._shared = adapter
    green = np.empty((8, 8, 3), np.uint8)
    green[:] = (20, 225, 60)
    image = raster(green)
    positions = [(-.8, -.8, 0.), (.8, -.8, 0.), (.8, .8, .1), (-.8, .8, .1)]
    uvs = [(0, 1), (1, 1), (1, 0), (0, 0)]
    triangles = [(0, 1, 2), (0, 2, 3)]
    if layer == 'image':
        green[:4] = (230, 55, 10)                        # the top half
        path = tmp_path / 'plan.png'
        assert raster(green).save(str(path))
        scene.image_planes.append(ImagePlane(str(path), Vec3(-.8, -.8, 0.),
                                             Vec3(1.6, 0, 0), Vec3(0, 1.6, 0)))
    elif layer == 'map':
        scene.georef = SceneDatum(-12.1, -77.05)
        tiles = scene.tile_layer = TileLayer(next(iter(PRESETS.values())), radius_m=4)
        for x, y in tiles.flat_tiles(scene.georef):
            tiles.images[(x, y, tiles.zoom)] = image
    elif layer == 'terrain':
        terrain = scene.terrain = TerrainObject([Vec3(*p) for p in positions],
                                                uvs, triangles, (0, 0, 1, 1, 16))
        terrain.texture_image = image
    else:
        survey = scene.photo_mesh = PhotoMesh(np.array(positions), np.array(uvs),
                                              np.array(triangles), [PhotoMaterial('scan', None, 0, 2)])
        survey.images[0] = image
    sync(adapter, style)
    live = capture(native)
    view = Matrix4()
    view.translate(0, 0, -3)
    projection = Matrix4()
    projection.ortho(-1, 1, -1, 1, .1, 10)
    pixels = native.render_image(64, 64, view.data(), projection.data(), style)
    export = np.frombuffer(bytes(pixels), np.uint8).reshape(64, 64, 4)
    for frame in (live, export):
        green = frame[46, 32, :3].astype(int)
        assert green[1] > 180 and green[0] < 40, green
        if layer == 'image':
            assert frame[18, 32, 0] > 200
    if layer == 'image':
        scene.image_planes.clear()
    else:
        {'terrain': scene.terrain, 'map': scene.tile_layer,
         'photo': scene.photo_mesh}[layer].visible = False
    sync(adapter, style)
    np.testing.assert_array_equal(capture(native), baseline)


def test_acquired_center_cue_projected_into_live_frame(native):
    upload(native, 255)
    native.set_style(dict(sky=False))
    native.set_overlay(64, 64, np.zeros(64 * 64 * 4, np.uint8))
    baseline = capture(native)
    native.set_center_cue((0, 0, 0), .9, (45, 32), 1.)
    near = capture(native)
    np.testing.assert_allclose(near[32, 32, :3], (41, 158, 92), atol=2)
    assert np.all(near[32, 37, :3] > baseline[32, 37, :3] - 1.) or near[32, 37, 2] > 100
    for center, radius, cursor in [((0, 0, 0), .9, (52, 32)),
                                   ((0, 0, 0), .1, (32, 32)),
                                   (None, .9, (32, 32))]:
        native.set_center_cue(center, radius, cursor, 1.)
        np.testing.assert_array_equal(capture(native), baseline)


@pytest.mark.parametrize("antialias", [False, True])
def test_model_behind_reference_only_hidden_by_opaque_imagery(native, antialias):
    upload(native, alpha=255)
    native.set_antialias(antialias)
    native.set_style(dict(sky=False, edges=False))
    original = capture(native)
    corners = [(-.8, -.8), (.8, -.8), (.8, .8), (-.8, .8)]
    vertices = np.array([(x, y, .25, 0., 0.) for x, y in
                         [corners[i] for i in (0, 1, 2, 0, 2, 3)]], np.float32).ravel()
    green = np.array([20, 225, 60, 255], np.uint8)
    def frame(opacity, depth):
        layer = ('reference', vertices, 1, 1, green, opacity, (0., 0., 0., 0.), depth)
        native.set_reference_layers(['reference'], [layer])
        return capture(native)[32, 32]
    np.testing.assert_allclose(frame(1., True)[:3], green[:3], atol=2)
    # Flat tiles and faded images leave geometry visible behind their plane.
    np.testing.assert_array_equal(frame(1., False), original[32, 32])
    np.testing.assert_array_equal(frame(.5, False), original[32, 32])


def test_selection_marks_follow_the_grab_and_leave_hover_marks_in_place(native):
    from ingetrazo_core import Vec3
    vertices, *_ = upload(native, 255)
    native.set_style(dict(sky=False, edges=False))
    marks = np.tile(vertices[:6], 2)
    marks['pos'][:, :2] *= .3
    marks['pos'][:6, 0] -= .45
    marks['pos'][6:, 1] += .6
    marks['pos'][:, 2] = .1
    marks['color'][:] = [0, 255, 0, 255]
    native.set_face_marks(marks.view(np.uint8).ravel())
    before = capture(native)
    assert np.count_nonzero(before[24:40, 7:26, 1] > 150) > 4

    scene = Scene()
    edge = scene.mesh.add_edge(Vec3(-.65, -.5, .1), Vec3(-.25, -.5, .1))
    native.begin_tool_marks({edge}, None, {edge.v0.key, edge.v1.key})
    matrix = np.eye(4)
    matrix[3, 0] = .9
    native.pose_tool_marks(matrix.ravel(), 6)
    moved = capture(native)
    assert not np.any(moved[24:40, 7:26, 1] > 150)
    assert np.count_nonzero(moved[24:40, 38:58, 1] > 150) > 4
    assert np.count_nonzero(moved[46:50, 38:58, 1] > 50) > 4
    np.testing.assert_array_equal(before[:20], moved[:20])
    native.end_tool_marks()
    np.testing.assert_array_equal(before, capture(native))


@pytest.mark.parametrize('aa', [False, True])
def test_clipboard_group_preview_moves_nested_texture_and_preserves_scene(native, tmp_path, aa):
    from types import SimpleNamespace
    from ingetrazo_core import Vec3, Matrix4
    from core.scene import Scene
    from core.group import Group
    from core.mesh import Mesh
    from ingetrazo_render import MeshToolAccess

    upload(native, 255)
    native.set_antialias(aa)
    native.set_style(dict(sky=False, edges=False))
    original = capture(native)
    path = str(tmp_path / 'clipboard.png')
    assert filled(2, 1, (0, 255, 100)).save(path)
    mesh = Mesh()
    face = mesh.add_face([Vec3(x, y, .3) for x, y in
                          [(-.3, -.3), (.3, -.3), (.3, .3), (-.3, .3)]])
    face.attrs = {'texture': {'path': path}}
    child = Group(mesh)
    root = Group(Mesh())
    root.children.append(child)
    placement = Matrix4()
    placement.translate(Vec3(-.5, 0, 0))
    root.xform = placement
    scene = Scene()
    access = MeshToolAccess()
    access.scene = scene
    access.update = lambda: None
    access.view = SimpleNamespace(renderer=native, packed=SimpleNamespace(origin=(0., 0., 0.)))
    version = scene.version
    access.begin_groups_preview([root], external=True)
    before = capture(native)
    access.set_groups_preview_offset(Vec3(1., 0, 0))
    after = capture(native)
    access.end_groups_preview()
    np.testing.assert_array_equal(capture(native), original)
    for frame, on, off in [(before, 16, 48), (after, 48, 16)]:
        assert frame[32, on, 1] > 220 and frame[32, on, 0] < 20
        np.testing.assert_array_equal(frame[32, off], original[32, off])
    assert list(root.children) == [child] and list(scene.groups) == []
    assert scene.version == version
