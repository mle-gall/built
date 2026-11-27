# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Built contributors.
"""Document identities survive edits and saves; placements remain distinct."""
import built_core
import re
import uuid

from built_core import Matrix4 as QMatrix4x4, Vec3 as QVector3D

from built_core import layers as L
from core.bim import tag_group, tag_faces
from built_core.group import Group, copy_group
from built_core.mesh import Mesh
from built_core.scene import Scene
from core.ifc_identity import component_guid, compressed_guid, copied_face_attrs
from formats import igz, clip_payload as clip
from formats.ifc import save_ifc


def mesh():
    m = Mesh()
    m.add_face([QVector3D(0, 0, 0), QVector3D(1, 0, 0), QVector3D(0, 1, 0)])
    return m


def guids(scene, path):
    save_ifc(scene, path)
    return re.findall(r"=IFC\w+\('([0-3][\w$]{21})'", path.read_text())


def test_guid_encoding():
    assert compressed_guid(uuid.UUID(int=0)) == '0' * 22
    assert compressed_guid(uuid.UUID(int=(1 << 128) - 1)) == '3' + '$' * 21


def test_export_copy_reload_and_legacy(tmp_path):
    s = Scene()
    g = Group(mesh())
    g.ifc = {"class": "IfcWall", "name": "Legacy"}
    s.groups.append(g)
    path = tmp_path / 'model.ifc'
    version = s.content_version
    first = guids(s, path)
    assert s.content_version > version
    version = s.content_version
    assert guids(s, path) == first
    assert s.content_version == version
    # The GlobalId derives from the placement's persistent uid path; the
    # export stores no GUID in the record (docs/persistent-ids.md §7.1).
    assert 'guid' not in g.ifc
    assert component_guid(s, g.uid) in first
    igz.save_scene(s, tmp_path / 'model.igz')
    loaded = Scene()
    igz.load_into(loaded, tmp_path / 'model.igz')
    assert guids(loaded, path) == first
    duplicate = copy_group(g)
    s.groups.append(duplicate)
    copied = guids(s, path)
    assert component_guid(s, duplicate.uid) != component_guid(s, g.uid)
    assert len(copied) == len(set(copied))
    tag_group(g, 'IfcSlab', 'Renamed')
    assert component_guid(s, g.uid) in first


def test_shared_face_tags_are_scoped_to_placements(tmp_path):
    s = Scene()
    g = Group(mesh())
    g.xform = QMatrix4x4()
    tag_faces(g.mesh.faces, 'IfcWall', 'Wall', 1)
    s.groups = [g, copy_group(g)]
    assert s.groups[0].mesh is s.groups[1].mesh
    first = guids(s, tmp_path / 'a.ifc')
    assert len(first) == len(set(first))
    assert g.mesh.faces[0].attrs['ifc']['guid']
    assert guids(s, tmp_path / 'b.ifc') == first
    igz.save_scene(s, tmp_path / 'a.igz')
    other = Scene()
    igz.load_into(other, tmp_path / 'a.igz')
    assert guids(other, tmp_path / 'c.ifc') == first


def test_nested_equal_uids_give_shared_face_tags_distinct_ifc_ids(tmp_path):
    from built_core.deliveries import scoped
    scene = Scene()
    part = mesh()
    tag_faces(part.faces, 'IfcWall', 'Wall', 1)
    for name in ('west', 'east'):
        child = Group(part, uid='repeated')
        child.xform = QMatrix4x4()
        parent = Group(uid=name)
        parent.adopt([child])
        scene.groups.append(parent)
    ids = guids(scene, tmp_path / 'full.ifc')
    assert len(ids) == len(set(ids))
    first = set(re.findall(r"=IFCWALL\('([^']+)'", (tmp_path / 'full.ifc').read_text()))
    assert len(first) == 2
    scene.select([scene.groups[1].children[0]])
    subset = scoped(scene, {'scope': 'selection', 'format': 'ifc'})
    guids(subset, tmp_path / 'one.ifc')
    only, = re.findall(r"=IFCWALL\('([^']+)'", (tmp_path / 'one.ifc').read_text())
    assert only in first


def test_face_copy_remaps_whole_objects():
    attrs = {'ifc': {'id': 1, 'guid': 'old', 'class': 'IfcWall'}}
    a, b = copied_face_attrs(), copied_face_attrs()
    assert a(attrs) == a(attrs)
    assert a(attrs)['ifc']['id'] != b(attrs)['ifc']['id']
    assert a(attrs)['ifc']['guid'] != 'old'
    assert attrs['ifc']['guid'] == 'old'


def test_layer_folder_rename_save_and_template(tmp_path):
    s = Scene()
    f = built_core.tag_edits.add_folder(s, 'Folder')
    s.layers.append(L.Layer('Wall', folder=f))
    data = built_core.tag_edits.template_of(s)
    uid = s.layer('Wall').uid
    fuid = s.layer_folder_uids[f]
    assert L.rename_layer(s, 'Wall', 'Walls')
    assert built_core.tag_edits.rename_folder(s, f, 'Architecture')
    assert s.layer('Walls').uid == uid
    # A rename relabels: the folder keeps its key, so its uid stays put.
    assert s.layer_folder_uids[f] == fuid and built_core.tag_edits.folder_label(s, f) == 'Architecture'
    assert built_core.tag_edits.apply_template(s, data)[0] == 0
    assert s.layer('Walls').folder == f
    assert built_core.tag_edits.add_missing(s, data) == ([], [])
    igz.save_scene(s, tmp_path / 'layers.igz')
    other = Scene()
    igz.load_into(other, tmp_path / 'layers.igz')
    assert [ly.uid for ly in other.layers] == [ly.uid for ly in s.layers]
    assert other.layer_folder_uids == s.layer_folder_uids
    collision = Scene()
    collision.layers.append(L.Layer('Wall'))
    target_uid = collision.layer('Wall').uid
    built_core.tag_edits.add_missing(collision, data)
    assert collision.layer('Wall').uid == target_uid


def test_clipboard_keeps_tag_identity_and_paste_resolves_renames():
    from tools.paste import PasteTool
    from built_core.history import History
    from types import SimpleNamespace
    source = Scene()
    source.layers.append(L.Layer('Old'))
    face = mesh().faces[0]
    payload = {'faces': [(face.vertices, [], {'layer': 'Old'})], 'edges': [],
               'groups': [], 'ref': QVector3D(), 'layers': built_core.tag_edits.template_for(source, ['Old'])}
    payload = clip.decode(clip.encode(payload))
    target = Scene()
    target.layers.append(L.Layer('New', uid=source.layer('Old').uid))
    vp = SimpleNamespace(scene=target, history=History(target), clipboard=payload,
                         update=lambda: None)
    assert PasteTool.in_place(vp)
    assert target.mesh.faces[0].attrs['layer'] == 'New'
    assert target.layer('Old') is None


def test_legacy_file_gets_identities_on_export(tmp_path):
    s = Scene()
    g = Group(mesh())
    g.ifc = {'class': 'IfcWall'}
    s.groups.append(g)
    igz.save_scene(s, tmp_path / 'legacy.igz')
    loaded = Scene()
    igz.load_into(loaded, tmp_path / 'legacy.igz')
    assert 'guid' not in loaded.groups[0].ifc
    assert 'ifc_identity' not in loaded.plugin_data
    out = guids(loaded, tmp_path / 'out.ifc')
    assert component_guid(loaded, loaded.groups[0].uid) in out
    assert all(k in loaded.plugin_data['ifc_identity']
               for k in ('project', 'site', 'building', 'storey'))


def test_undo_added_tags_uses_identity_after_rename():
    from built_core.history import EnsureLayersCommand
    src, dst = Scene(), Scene()
    folder = built_core.tag_edits.add_folder(src, 'Folder')
    src.layers.append(L.Layer('Wall', folder=folder))
    command = EnsureLayersCommand(built_core.tag_edits.template_of(src))
    command.do(dst)
    L.rename_layer(dst, 'Wall', 'Walls')
    built_core.tag_edits.rename_folder(dst, folder, 'Building')
    command.undo(dst)
    assert dst.layer('Walls') is None
    assert not L.folders_of(dst)
    assert not dst.layer_folder_uids
