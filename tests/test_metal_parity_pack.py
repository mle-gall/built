"""P3 pack ABI: GL resolution, hidden streams and bounded texture mips."""
from tests.raster_samples import solid_raster
from pathlib import Path
from types import SimpleNamespace

import numpy as np
import pytest
from built_core import Vec3 as QVector3D, Matrix4 as QMatrix4x4
from core import _native
from core.group import Group
from core.layers import Layer
from core.materials import effective_attrs
from core.mesh import Mesh
from core.scene import Scene
from views.document_interaction import _paint_of
from built_core import pack_scene, source_arrays, COMMAND, OBJECT_HIDDEN, VISIBILITY_FILTERED
from built_render import material_texture_array


def triangle(attrs=None):
    m = Mesh()
    m.add_face([QVector3D(0, 0, 0), QVector3D(2, 0, 0), QVector3D(0, 3, 0)])
    m.faces[0].attrs.update(attrs or {})
    return m


def face_colors(packed, prototype=0):
    cmd = packed.commands[prototype]
    start = int(cmd['first'])
    return packed.vertices['color'][start:start+int(cmd['count'])]


@pytest.mark.parametrize('own,container,opacity', [
    ({'color': (.2, .4, .6), 'opacity': .3}, None, 1),
    ({}, {'color': (.2, .4, .6), 'opacity': .4}, 1),
    ({'color': (.2, .4, .6), 'opacity': .7}, None, .25),
    ({'color': (.2, .4, .6), 'opacity': .7}, {'opacity': .2}, .4),
    ({'color': (.2, .4, .6), 'opacity': 0}, None, 1),
    ({'color': (.2, .4, .6), 'opacity': 254/255}, None, 1),
])
def test_opacity_matches_gl(own, container, opacity):
    g = Group(triangle(own)); g.material = container; g.layer = 'tag'
    scene = Scene(groups=[g]); scene.layers.append(Layer('tag', opacity=opacity))
    expected = effective_attrs(own, _paint_of(SimpleNamespace(scene=scene), g))
    packed = pack_scene(scene)
    assert np.all(face_colors(packed)[:, 3] == max(1, round(expected.get('opacity', 1)*255)))


def test_locked_rgb_and_matching_back():
    attrs = {'color': (.2, .4, .6), 'back': True}
    g = Group(triangle(attrs)); g.layer = 'locked'
    scene = Scene(groups=[g]); scene.layers.append(Layer('locked', locked=True))
    expected = effective_attrs(attrs, _paint_of(SimpleNamespace(scene=scene), g))
    packed = pack_scene(scene)
    rgb = (np.array(expected['color'])*255).astype(np.uint8)
    np.testing.assert_array_equal(face_colors(packed)[:, :3], np.tile(rgb, (3, 1)))
    assert np.all(face_colors(packed)[:, 3] == 255)
    header = packed.materials[:16].view('<u4')
    assert header[2] == 0 and header[3] == 1  # matching backs reuse vertex RGBA
    assert packed.materials.nbytes == 16
    assert scene._layer_state(g)[1]
    assert not scene.entity_selectable(g)


def test_hidden_ranges_and_object_filter_bits():
    m = triangle({'hidden': True}); m.edges[0].hidden = True
    g = Group(m); g.hidden = True
    child = Group(triangle()); g.children = [child]
    scene = Scene(groups=[g]); packed = pack_scene(scene)
    assert packed.commands[0]['count'] == 0
    assert packed.hidden_commands.dtype == packed.hidden_edge_commands.dtype == COMMAND
    assert packed.hidden_commands[0]['count'] == 3
    assert packed.hidden_edge_commands[0]['count'] == 2
    assert np.all(packed.instances['info'][:, 2] & OBJECT_HIDDEN)
    assert not np.any(packed.instances['info'][:, 2] & VISIBILITY_FILTERED)
    g.layer = 'off'; scene.layers.append(Layer('off', visible=False))
    filtered = pack_scene(scene)
    assert np.all(filtered.instances['info'][:, 2] & VISIBILITY_FILTERED)


def test_no_hidden_visible_stream_is_byte_identical_to_legacy_kernel():
    mesh = triangle({'color': (.2, .4, .6)})
    arrays = source_arrays(mesh)
    offsets = np.array([[0]*5, [len(a) for a in arrays[:5]]], np.int64)
    raw, indices, bounds, table, materials = _native.available().bake_scene(
        *arrays[:4], arrays[6], arrays[7], arrays[4], arrays[5], offsets,
        arrays[8], _native.qt_hypot(), arrays[9], arrays[10])
    packed = pack_scene(Scene(groups=[Group(mesh)]))
    assert packed.vertices.tobytes() == raw.tobytes()
    assert packed.indices.tobytes() == indices.tobytes()
    assert packed.materials.tobytes() == materials.tobytes()
    first, nf, ne = table
    expected = np.zeros(1, COMMAND); expected['first'] = expected['base'] = first
    expected['count'] = nf
    assert packed.commands.tobytes() == expected.tobytes()
    expected['first'] = first+nf; expected['count'] = ne
    assert packed.edge_commands.tobytes() == expected.tobytes()
    assert not packed.hidden_commands['count'].any()
    assert not packed.hidden_edge_commands['count'].any()


def test_texture_resolution_budget_mips_and_ids(tmp_path):
    path = str(tmp_path/'large.png')
    image = solid_raster(2048, 32, (18, 52, 86)); image.save(path)
    w, h, layers, mips = material_texture_array([path])
    assert (w, h, layers) == (2048, 32, 2)
    assert len(mips[-1]) == 8
    assert mips[0][:w*h*4] == b'\xff'*(w*h*4)
    w, h, layers, mips = material_texture_array([path], device_limit=4096, memory_budget=16384)
    assert sum(map(len, mips)) <= 16384
    assert len(mips) == max(w, h).bit_length()
    assert material_texture_array([path], device_limit=512)[0] == 512
    with pytest.raises(ValueError):
        material_texture_array([path], memory_budget=7)


def test_audit_fixture_matches_gl_and_retains_hidden():
    from formats.igz import load_into
    scene = Scene(); load_into(scene, Path('docs/benchmarks/metal-parity/synthetic.igz'))
    packed = pack_scene(scene)
    for name in ('Translucent tag', 'Locked tag', 'Mirrored glass'):
        g = next(g for g in scene.groups if g.name == name)
        row = packed.uids.index(g.uid); proto = int(packed.instances['info'][row, 0])
        expected = effective_attrs(g.mesh.faces[0].attrs, _paint_of(SimpleNamespace(scene=scene), g))
        rgba = [*(np.array(expected['color'][:3])*255).astype(np.uint8),
                max(1, round(expected.get('opacity', 1)*255))]
        np.testing.assert_array_equal(face_colors(packed, proto), np.tile(rgba, (36, 1)))
    assert packed.hidden_commands['count'].sum() == 3
    assert packed.hidden_edge_commands['count'].sum() >= 2
    hidden = next(g for g in scene.groups if g.name == 'Hidden')
    assert packed.instances['info'][packed.uids.index(hidden.uid), 2] & OBJECT_HIDDEN


def test_incremental_hidden_change_rebakes_hidden_ranges_without_repack():
    from built_core import InstanceUpdates
    from tests.test_wgpu_updates import assert_matches_fresh_pack
    g = Group(triangle()); scene = Scene(groups=[g]); updates = InstanceUpdates(scene, pack_scene(scene))
    g.mesh.faces[0].attrs['hidden'] = True
    g.mesh.move_vertex(g.mesh.vertices[0], QVector3D(0, 0, 1))
    updates.apply(*updates.diff(scene))
    code = int(updates.packed.instances['info'][updates.resident.row(g.uid), 0])
    assert updates.packed.hidden_commands[code]['count'] == 3
    assert updates.packed.commands[code]['count'] == 0
    assert_matches_fresh_pack(updates, scene)


def test_incremental_paint_alpha_and_hidden_transform():
    from built_core import InstanceUpdates
    g = Group(triangle({'color': (1, 0, 0)})); scene = Scene(groups=[g])
    updates = InstanceUpdates(scene, pack_scene(scene))
    g.mesh.faces[0].attrs['opacity'] = .3
    g.mesh.move_vertex(g.mesh.vertices[0], QVector3D(0, 0, 1))
    updates.apply(*updates.diff(scene))
    assert np.all(face_colors(updates.packed)[:, 3] == 76)
    g.mesh.faces[0].attrs['hidden'] = True
    updates = InstanceUpdates(scene, pack_scene(scene))
    before = updates.packed.hidden_commands.tobytes()
    g.xform = QMatrix4x4()
    g.xform.translate(1, 0, 0)
    updates.apply(*updates.diff(scene))
    assert updates.packed.hidden_commands.tobytes() == before


def test_texture_only_opacity_and_distinct_back():
    arrays = source_arrays(triangle({'texture': {'path': 'fake.png'}, 'opacity': .2,
                                     'back': {'color': (1, 0, 0), 'opacity': .4}}))
    assert arrays[6][1, 3] == 51
    assert int(arrays[10][0, 0]) >> 24 == 102
    textured_back = source_arrays(triangle({
        'back': {'texture': {'path': 'back.png'}, 'opacity': .4}}))
    assert int(textured_back[10][0, 0]) >> 24 == 102
    assert textured_back[10][0, 2] == 1


def test_hidden_material_corners_follow_visible_edge_gap():
    mesh = triangle({'color': (1, 0, 0), 'back': {'color': (0, 0, 1)}})
    f = mesh.add_face([QVector3D(4, 0, 0), QVector3D(6, 0, 0), QVector3D(4, 3, 0)])
    f.attrs.update(hidden=True, color=(0, 1, 0), opacity=.4, back=True)
    packed = pack_scene(Scene(groups=[Group(mesh), Group(triangle())]))
    cmd = packed.hidden_commands[0]
    first = int(cmd['first'])
    assert first == int(packed.edge_commands[0]['first'] + packed.edge_commands[0]['count'])
    np.testing.assert_array_equal(packed.vertices['color'][first:first+3], [[0, 255, 0, 102]]*3)
    header = packed.materials[:16].view('<u4')
    at = int(header[1])*16 + (first-int(header[0]))*32
    np.testing.assert_array_equal(packed.materials[at:at+96].reshape(3, 32)[:, 16:20],
                                  [[0, 255, 0, 102]]*3)
    # Index values remain prototype-relative; every range shares its base.
    assert int(packed.commands[1]['base']) == len(packed.vertices)-9
    for commands in (packed.commands, packed.edge_commands,
                     packed.hidden_commands, packed.hidden_edge_commands):
        for command in commands:
            first, count, base = map(int, (command['first'], command['count'], command['base']))
            assert np.all(packed.indices[first:first+count] + base < len(packed.vertices))


def test_native_material_palette_matches_numpy():
    """Quantization must preserve truncation, opacity rounding and both sides."""
    native = _native.available()
    # (r, g, b, opacity byte) of each attribute row's back, and its front
    # and back texture layers: what the pack hands the palette kernel.
    colors = [None, (-.1, .499, 1.2, 128), (.2, .3, .4, 26), (210/255,)*3 + (255,)]
    slots = [[0, 0], [0, 0], [0, 0], [1, 1]]
    attributes = np.array([3, 0, 2, 1, 1], dtype=np.int64)
    actual = native.pack_materials(colors, slots, attributes)
    palette = np.zeros((len(colors), 3), np.uint32)
    for k, color in enumerate(colors):
        if color is not None:
            palette[k, 0] = np.array([*np.clip(np.array(color[:3])*255, 0, 255), color[3]], np.uint8).view('<u4')[0]
    palette[:, 1:] = slots
    expected = palette[attributes]
    assert actual.dtype == expected.dtype and actual.shape == expected.shape
    assert actual.tobytes() == expected.tobytes()
    with pytest.raises(ValueError):
        native.pack_materials([None], [[0,0]], np.array([-1], np.int64))


def test_mixed_material_pack_matches_python_palette(monkeypatch, tmp_path):
    import built_core
    path = str(tmp_path / 'swatch.png')
    image = solid_raster(2, 2, (255, 0, 0))
    assert image.save(path)
    mesh = Mesh()
    for x, paint in enumerate([
        {'color': (.3, .6, .9)},
        {'texture': {'path': path}},
        {'pbr': {'metallic': .7}, 'back': {'pbr': {'roughness': .2}}},
        {},
    ]):
        face = mesh.add_face([QVector3D(x, 0, 0), QVector3D(x+1, 0, 0), QVector3D(x, 1, 0)])
        face.attrs.update(paint)
    scene = Scene(groups=[Group(mesh)])
    overrides = {face.key: {'back': {'color': (.21, .42, .73), 'opacity': .66}}}
    native = _native.available()
    assert hasattr(native, 'pack_materials')
    monkeypatch.setattr(_native, 'native', native)
    actual = built_core.pack_scene(scene, face_overrides=overrides)
    # Disable only the new kernel so every other pack stage stays identical.
    class PreviousCore:
        def __getattr__(self, name):
            if name == 'pack_materials':
                raise AttributeError(name)
            return getattr(native, name)
    monkeypatch.setattr(_native, 'native', PreviousCore())
    expected = built_core.pack_scene(scene, face_overrides=overrides)
    assert actual.materials.tobytes() == expected.materials.tobytes()
    assert actual.vertices.tobytes() == expected.vertices.tobytes()
    assert actual.texture_paths == expected.texture_paths
    assert actual.textures == expected.textures
