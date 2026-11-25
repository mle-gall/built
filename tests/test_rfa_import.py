# SPDX-License-Identifier: GPL-3.0-or-later
import base64
from copy import deepcopy
import json
import sys
from types import SimpleNamespace

import pytest
from tests.shell_isolation import native_window, restore_shell_patches  # noqa: F401
from ingetrazo_render import SurveyRaster, component_pixels
from ingetrazo_core import Vec3 as QVector3D

from core.scene import Scene
from core.layers import Layer
from core.materials import Material
from core.kept_components import definitions_of
from core.bim_registry import get
from formats.rfa import apply_payload, read_rfa, RfaImport, ZONE_TAG_KEY


def payload():
    part = dict(solid=112, material='steel', style='clear', hidden=False, zone=False,
                positions=[0, 0, 0, 1, 0, 0, 1, 1, 0, 0, 1, 0], indices=[0, 1, 2, 0, 2, 3])
    return {'schema': 'ingetrazo.rfa/1', 'source': {'file_name': 'cabinet.rfa'},
            'family': {'name': 'Cabinet', 'category': 'Electrical Equipment', 'omniclass': '23.80.30.11.17'},
            'materials': {'steel': {'name': 'Steel', 'rgb': [250, 243, 227], 'transparency': 0}},
            'styles': {'clear': {'subcategory': 'Clearance'}},
            'geometries': {'g': {'label': 'Cabinet', 'parts': [part, dict(part, hidden=True), dict(part, solid=113, zone=True)]}},
            'types': [{'name': 'Standard', 'parameters': [{'name': 'Manufacturer', 'value': 'Schneider', 'kind': 'Text', 'units': None}],
                       'components': [{'geometry': 'g', 'transform': [0, -1, 0, 2, 1, 0, 0, 3, 0, 0, 1, 4, 0, 0, 0, 1]}],
                       'geometry_source': 'nested_type'},
                      {'name': 'Empty', 'parameters': [{'name': k, 'value': v, 'kind': 'Length', 'units': 'mm'} for k, v in [('Width', '710'), ('Depth', '600'), ('Height', '2000')]],
                       'components': [], 'geometry_source': 'none'}],
            'warnings': ['Decoder warning']}


def test_mapping():
    scene = Scene()
    # The zone tag is found by its key, whatever the user renamed it to.
    scene.layers.append(Layer('Dégagements', key=ZONE_TAG_KEY))
    scene.materials['Steel'] = Material('Steel', (250/255, 243/255, 227/255))
    data = payload()
    original = deepcopy(data)
    report = apply_payload(scene, data)
    assert data == original
    assert [g.name for g in scene.kept_components] == ['Standard', 'Empty']
    assert len(definitions_of(scene)) == 2
    assert len(scene.groups) == 1
    definition = scene.kept_components[0]
    assert scene.groups[0].mesh is definition.mesh
    assert scene.groups[0].xform.isIdentity()
    assert len(definition.children) == 2
    solid, zone = definition.children
    assert solid.name == 'Steel'
    assert len(solid.children) == 2
    assert [g.hidden for g in solid.children] == [False, True]
    assert len(solid.children[0].mesh.faces) == 2
    assert zone.layer == zone.children[0].layer == ZONE_TAG_KEY
    assert [ly.key for ly in scene.layers].count(ZONE_TAG_KEY) == 1
    assert zone.children[0].material['opacity'] == .2
    assert len(scene.materials) == 2
    assert solid.children[0].material['color'] == pytest.approx((250/255, 243/255, 227/255))
    assert solid.xform.map(QVector3D(1, 0, 0)).toTuple() == pytest.approx((2, 4, 4))
    assert max(v.position.x() for v in solid.children[0].mesh.vertices) == 1
    assert definition.ifc['classifications'] == [{'system': 'omniclass', 'code': '23.80.30.11.17'}]
    assert definition.ifc['psets']['Revit family']['Source file'] == 'cabinet.rfa'
    assert definition.ifc['psets']['Revit family']['Category'] == 'Electrical Equipment'
    assert definition.ext['rfa']['parameters'][0]['value'] == 'Schneider'
    box = scene.kept_components[1].children[0].mesh
    assert len(box.faces) == 6
    assert tuple(max(getattr(v.position, axis)() for v in box.vertices) for axis in ('x', 'y', 'z')) == pytest.approx((.710, .600, 2))
    assert scene.kept_components[1].ifc['psets']['Revit parameters']['Width'] == {'kind': 'length', 'value': .71}
    assert report[0] == 'Decoder warning'
    assert any('placeholder' in w for w in report)


def test_created_tag_has_key_and_undo_redo():
    scene = Scene()
    before_layers = list(scene.layers)
    before_registry = get(scene)
    cmd = RfaImport(lambda s: apply_payload(s, payload()))
    cmd.do(scene)
    assert 'omniclass' in get(scene)['classification_systems']
    tag = next(ly for ly in scene.layers if ly.name == 'Clearance zones')
    assert tag.key != tag.name
    cmd.undo(scene)
    assert not scene.groups and not scene.kept_components and not scene.materials
    assert scene.layers == before_layers
    assert get(scene) == before_registry
    cmd.do(scene)
    assert len(scene.kept_components) == 2 and len(scene.groups) == 1
    assert len(scene.materials) == 2


def test_missing_module(monkeypatch):
    monkeypatch.setitem(sys.modules, 'ingetrazo_rfa', None)
    with pytest.raises(RuntimeError, match='scripts/build_native.sh'):
        read_rfa('unused.rfa')


def test_reader_contract(monkeypatch):
    seen = []
    def read(path):
        seen.append(path)
        return json.dumps(payload())
    monkeypatch.setitem(sys.modules, 'ingetrazo_rfa', SimpleNamespace(read_family=read))
    assert read_rfa('test.rfa') == payload()
    assert seen == ['test.rfa']


def test_thumbnail():
    data = SurveyRaster(16, 16, bytes((255, 0, 0, 255)) * 256).png()
    p = payload()
    p['preview_png_base64'] = base64.b64encode(bytes(data)).decode()
    scene = Scene()
    apply_payload(scene, p)
    pixels = component_pixels(scene.kept_components[0], 64)
    offset = 4 * (20 * 64 + 20)
    assert pixels[offset:offset + 4] == bytes((255, 0, 0, 255))


def test_bad_indices_rollback():
    scene = Scene()
    data = payload()
    data['geometries']['g']['parts'][0]['indices'][0] = -1
    cmd = RfaImport(lambda s: apply_payload(s, data))
    with pytest.raises(ValueError, match='index'):
        cmd.do(scene)
    assert not scene.kept_components and not scene.groups and not scene.materials


def test_persistence(tmp_path):
    from formats.igz import save_scene, load_into
    scene = Scene()
    RfaImport(lambda s: apply_payload(s, payload())).do(scene)
    path = tmp_path / 'family.igz'
    save_scene(scene, path)
    restored = Scene()
    load_into(restored, path)
    assert len(definitions_of(restored)) == 2
    assert restored.kept_components[0].ext['rfa']['source']['file_name'] == 'cabinet.rfa'
    assert restored.groups[0].ifc['classifications'][0]['system'] == 'omniclass'
    assert restored.groups[0].children[0].children[1].hidden
    assert restored.layer(restored.groups[0].children[1].layer).name == 'Clearance zones'


def test_menu_dispatch_and_report(native_window, monkeypatch, tmp_path):
    from app_shell.files import IMPORTS
    from views.filedialogs import file_dialogs
    from formats import rfa

    win = native_window
    reports = []
    path = tmp_path / "test.rfa"
    path.touch()
    monkeypatch.setattr(rfa, "read_rfa", lambda path: payload())
    monkeypatch.setattr("ingetrazo_render.chrome_dialogs.information", lambda parent, title, text: reports.append(text))
    monkeypatch.setattr(file_dialogs, "getOpenFileName", lambda *args, **kwargs: (str(path), ""))
    assert ".rfa" in IMPORTS
    win._host.menu("import:rfa")
    assert len(win.viewport.scene.kept_components) == 2
    assert reports and "Decoder warning" in reports[0] and "placeholder" in reports[0]
    win.viewport.history.undo()
    assert not win.viewport.scene.kept_components
    assert win._host.open_path(path)
    assert len(win.viewport.scene.kept_components) == 2
