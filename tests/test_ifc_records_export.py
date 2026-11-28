"""Records survive export, with real schema and IDS consumers when installed."""
from pathlib import Path
import pytest
from built_core import Vec3 as QVector3D, Matrix4 as QMatrix4x4
from built_core.scene import Scene
from built_core.group import Group
from formats.ifc import save_ifc
from formats.ifc.entities import Builder
from formats.ifc.step import _s


def rich_scene(schema='IFC4'):
    scene = Scene()
    storey = Group(name='Deck')
    storey.ifc = {'class': 'IfcBuildingStorey'}
    storey.xform = QMatrix4x4()
    storey.xform.translate(0, 0, 7)
    assembly = Group(name='Ride')
    assembly.ifc = {'class': 'IfcElementAssembly'}
    transparent = Group()
    wall = Group(name='Enclosure')
    wall.mesh.add_face([QVector3D(0, 0, 0), QVector3D(2, 0, 0),
                        QVector3D(2, 0, 3), QVector3D(0, 0, 3)])
    wall.ifc = {
        'class': 'IfcWall', 'predefined_type': 'USERDEFINED', 'object_type': 'Ride wall',
        'description': 'Surveyed', 'tag': 'W-1', 'type': 'wall', 'systems': ['ride'],
        'classifications': [{'system': 'discipline', 'code': 'A', 'name': 'Architectural'}],
        'psets': {'Pset_WallCommon': {'Status': {'kind': 'enum', 'value': 'EXISTING'}},
                  'Survey': {'Note': "café 😀 \\ survey", 'Count': 3, 'Checked': True,
                             'Offset': {'kind': 'length', 'value': 0.2}}},
        'quantities': {'NetSideArea': {'value': 8, 'source': 'manual'}}}
    storey.children = [assembly]
    assembly.children = [transparent]
    transparent.children = [wall]
    scene.groups.append(storey)
    scene.plugin_data['bim_registry'] = {
        'classification_systems': {'discipline': {'name': 'Discipline', 'edition': '2026', 'source': 'Survey'}},
        'types': {'wall': {'ifc_class': 'IfcWall', 'name': 'Wall type', 'predefined_type': 'STANDARD',
                          'psets': {'Pset_WallCommon': {'Status': 'NEW', 'IsExternal': True}},
                          'classifications': [{'system': 'discipline', 'code': 'A', 'name': 'Architectural'}]}},
        'systems': {'ride': {'ifc_class': 'IfcSystem', 'name': 'Ride'}, 'unused': {'name': 'Unused'}},
        'project': {'author': 'Surveyor', 'organization': 'Owner'},
        'export_profiles': {'handover': {'schema': schema, 'view': 'ReferenceView'}}}
    return scene, wall


@pytest.mark.parametrize('schema', ['IFC4', 'IFC4X3_ADD2'])
def test_record_round_trip(tmp_path, schema):
    ifc = pytest.importorskip('ifcopenshell')
    import ifcopenshell.validate
    import ifcopenshell.util.element
    scene, wall = rich_scene(schema)
    path = tmp_path / 'ride.ifc'
    report = save_ifc(scene, path, profile='handover')
    assert report.count == 3
    assert not report.problems
    model = ifc.open(str(path))
    logger = ifcopenshell.validate.json_logger()
    ifcopenshell.validate.validate(model, logger, express_rules=True)
    assert not logger.statements
    element = model.by_type('IfcWall')[0]
    assert element.Name == 'Enclosure'
    assert element.Tag == 'W-1'
    assert element.ObjectType == 'Ride wall'
    assert element.PredefinedType == 'USERDEFINED'
    assert element.Description == 'Surveyed'
    psets = ifcopenshell.util.element.get_psets(element)
    assert psets['Pset_WallCommon']['Status'] == 'EXISTING'
    assert psets['Pset_WallCommon']['IsExternal'] is True
    assert psets['Survey']['Note'] == "café 😀 \\ survey"
    assert psets['Qto_WallBaseQuantities']['NetSideArea'] == 8
    assert next(q for q in model.by_type('IfcQuantityArea') if q.Name == 'NetSideArea').Description == 'manual'
    assert len(model.by_type('IfcSystem')) == 1
    assert len(model.by_type('IfcClassificationReference')) == 1
    deck = next(s for s in model.by_type('IfcBuildingStorey') if s.Name == 'Deck')
    assert deck.Elevation == 7
    assert element.Decomposes[0].RelatingObject.is_a('IfcElementAssembly')
    assert ifcopenshell.util.element.get_container(element) == deck
    ids = sorted(e.GlobalId for e in model.by_type('IfcRoot'))
    save_ifc(scene, path, profile='handover')
    assert ids == sorted(e.GlobalId for e in ifc.open(str(path)).by_type('IfcRoot'))


def test_attribute_counts_are_checked():
    with pytest.raises(ValueError):
        Builder('IFC4').add('IfcWall', '$')


def test_invalid_record_is_reported(tmp_path):
    scene, wall = rich_scene()
    wall.ifc['object_type'] = ''
    report = save_ifc(scene, tmp_path / 'invalid.ifc')
    assert report.count == 2
    assert any('USERDEFINED' in m for p in report.problems for m in p['messages'])


@pytest.mark.parametrize('passes', [True, False])
def test_ids_check(tmp_path, passes):
    pytest.importorskip('ifctester')
    scene, wall = rich_scene()
    scene.plugin_data['bim_registry']['export_profiles']['handover']['ids_files'] = [
        str(Path(__file__).parent / 'fixtures' / 'wall-status.ids')]
    if not passes:
        wall.ifc.pop('classifications')
        scene.plugin_data['bim_registry']['types']['wall'].pop('classifications')
    report = save_ifc(scene, tmp_path / 'ids.ifc', profile='handover')
    assert report.ids[0]['status'] == 'checked'
    spec = report.ids[0]['specifications'][0]
    assert spec['status'] == passes
    assert spec['passed'] == int(passes)
    assert spec['failed'] == int(not passes)
    assert (tmp_path / 'ids.ids-report.json').exists()


@pytest.mark.parametrize('schema', ['IFC4', 'IFC4X3_ADD2'])
def test_all_curated_occurrences_validate(tmp_path, schema):
    ifc = pytest.importorskip('ifcopenshell')
    import ifcopenshell.validate
    from core.ifc_tables import IFC_CLASSES
    scene = Scene()
    for cls in IFC_CLASSES:
        group = Group(name=cls)
        group.ifc = {'class': cls}
        scene.groups.append(group)
    scene.plugin_data['bim_registry'] = {'export_profiles': {'test': {'schema': schema}}}
    path = tmp_path / 'classes.ifc'
    save_ifc(scene, path, profile='test')
    logger = ifcopenshell.validate.json_logger()
    ifcopenshell.validate.validate(str(path), logger, express_rules=True)
    assert not logger.statements


def test_casita_and_legacy_validate_with_express_rules(tmp_path):
    ifc = pytest.importorskip('ifcopenshell')
    import ifcopenshell.validate
    from tests.test_ifc_bridge import _extrude, _Vp, V
    from core.bim import tag_faces, next_object_id
    scene = Scene()
    vp = _Vp(scene)
    for cls, x, width, depth, height in [
        ('IfcWall', 0, 4, .15, 2.6), ('IfcWall', 5, 4, .15, 2.6),
        ('IfcSlab', 10, 4, 3, .2), ('IfcColumn', 15, .25, .25, 3),
        ('IfcPile', 24, .3, .3, 8)]:
        faces = _extrude(scene, vp, [V(x, 0), V(x+width, 0),
                                    V(x+width, depth), V(x, depth)], height)
        tag_faces(faces, cls, cls, next_object_id(scene))
    door = scene.mesh.add_face([V(30, 0, 0), V(30.9, 0, 0), V(30.9, 0, 2.1), V(30, 0, 2.1)])
    tag_faces([door], 'IfcDoor', 'Door', next_object_id(scene))
    path = tmp_path / 'casita.ifc'
    assert save_ifc(scene, path) == 6
    logger = ifcopenshell.validate.json_logger()
    ifcopenshell.validate.validate(str(path), logger, express_rules=True)
    assert not logger.statements


@pytest.mark.parametrize('schema', ['IFC4', 'IFC4X3_ADD2'])
def test_curated_types_validate(tmp_path, schema):
    pytest.importorskip('ifcopenshell')
    import ifcopenshell.validate
    from core.ifc_tables import IFC_CLASSES
    from core.record import role
    scene = Scene()
    types = {}
    for cls in IFC_CLASSES:
        if role({'class': cls}) == 'spatial' and cls not in ('IfcSpace', 'IfcSpatialZone'):
            continue
        group = Group(name=cls)
        group.ifc = {'class': cls, 'type': cls}
        types[cls] = {'ifc_class': cls, 'name': cls}
        scene.groups.append(group)
    scene.plugin_data['bim_registry'] = {
        'types': types, 'export_profiles': {'test': {'schema': schema}}}
    path = tmp_path / 'types.ifc'
    save_ifc(scene, path, profile='test')
    logger = ifcopenshell.validate.json_logger()
    ifcopenshell.validate.validate(str(path), logger, express_rules=True)
    assert not logger.statements


def test_generic_manual_area_replaces_the_measured_value(tmp_path):
    ifc = pytest.importorskip('ifcopenshell')
    scene, wall = rich_scene()
    wall.ifc['quantities'] = {'Area': {'value': 12, 'source': 'manual'}}
    path = tmp_path / 'manual.ifc'
    save_ifc(scene, path)
    model = ifc.open(str(path))
    values = {q.Name: q for q in model.by_type('IfcQuantityArea')}
    assert 'Area' not in values
    assert values['NetSideArea'].AreaValue == 12
    assert values['NetSideArea'].Description == 'manual'


def test_junction_products_and_realizers_survive_release(tmp_path):
    ifc = pytest.importorskip('ifcopenshell')
    import ifcopenshell.validate
    from ifcopenshell.util.element import get_psets
    from tests.test_live_junction import knee_fixture, apply
    from core.bim import junction_schedules

    scene, history, uid = knee_fixture()
    path = tmp_path / 'knee.ifc'
    def export():
        report = save_ifc(scene, path)
        assert not report.problems
        return ifc.open(str(path))

    model = export()
    ids = sorted(e.GlobalId for e in model.by_type('IfcRoot'))
    assert len(ids) == len(set(ids))
    assert ids == sorted(e.GlobalId for e in export().by_type('IfcRoot'))
    assembly, = model.by_type('IfcElementAssembly')
    assert assembly.PredefinedType == 'ACCESSORY_ASSEMBLY'
    assert assembly.IsTypedBy[0].RelatingType.Name == 'End plate'
    parts = assembly.IsDecomposedBy[0].RelatedObjects
    assert len(parts) == 6
    assert {e.is_a() for e in parts} == {'IfcPlate', 'IfcFastener', 'IfcMechanicalFastener'}
    fastener, = model.by_type('IfcMechanicalFastenerType')
    assert fastener.NominalDiameter == .016 and len(fastener.RepresentationMaps) == 1
    seam, = model.by_type('IfcFastener')
    assert get_psets(seam)['Built_Junction']['DetailName'] == 'Fillet weld'
    assert seam.Representation.Representations[0].RepresentationIdentifier == 'Axis'
    connections = model.by_type('IfcRelConnectsWithRealizingElements')
    assert sorted(len(c.RealizingElements) for c in connections) == [1, 4, 6]
    def realizing_links(file):
        return {c.GlobalId: (c.RelatingElement.GlobalId, c.RelatedElement.GlobalId,
                             frozenset(p.GlobalId for p in c.RealizingElements))
                for c in file.by_type('IfcRelConnectsWithRealizingElements')}
    holes = model.by_type('IfcVoidingFeature')
    assert len(holes) == 8 and all(h.PredefinedType == 'HOLE' for h in holes)
    assert all(h.Representation.Representations[0].Items[0].SweptArea.is_a('IfcCircleProfileDef') for h in holes)
    plate, = model.by_type('IfcPlate')
    assert any(rep.RepresentationType == 'SweptSolid' for rep in plate.Representation.Representations)
    logger = ifcopenshell.validate.json_logger()
    ifcopenshell.validate.validate(str(path), logger, express_rules=True)
    assert not logger.statements
    schedules = junction_schedules(scene)
    assert schedules['fasteners'][1] == [16, 4, 50, 'ISO 4017', '8.8', 'End plate', 1]
    assert schedules['welds'][1][2] == pytest.approx(.38)
    assert schedules['plates'][1][4] == 1
    assert schedules['plates'][1][5] == pytest.approx(195 * 190 * 12 * 7850 / 1e9)
    assert sum(r[-1] for r in schedules['holes'][1:]) == 8
    scene.plugin_data['bim_registry'].setdefault('export_profiles', {})['ref'] = {'view': 'ReferenceView'}
    reference = tmp_path / 'reference.ifc'
    assert not save_ifc(scene, reference, profile='ref').problems
    ref_model = ifc.open(str(reference))
    assert not ref_model.by_type('IfcVoidingFeature')
    assert ref_model.by_guid(plate.GlobalId).Representation.Representations[0].RepresentationType == 'Tessellation'
    apply(history, 'junction_release', uid=uid)
    released = export()
    assert ids == sorted(e.GlobalId for e in released.by_type('IfcRoot'))
    assert realizing_links(released) == realizing_links(model)
    assert schedules == junction_schedules(scene)
    from formats import igz
    igz.save_scene(scene, tmp_path / 'released.built')
    scene = Scene()
    igz.load_into(scene, tmp_path / 'released.built')
    assert realizing_links(export()) == realizing_links(model)
    assert schedules == junction_schedules(scene)


def garage_junctions():
    """Frame-8 connection quantities on 56 explicitly bound synthetic meetings."""
    from copy import deepcopy
    from tests.test_live_junction import knee_fixture, live, apply
    from tests.test_sketch_rows import member
    from built_core.history import History

    template, _, _ = knee_fixture()
    details = {p['name']: p for p, _ in live.preset_catalog(template)}
    knee = deepcopy(details['End plate'])
    knee['values']['contents'][1]['part']['fields']['profile.params.height'] = 550
    cleat = deepcopy(knee)
    cleat.update(uid='c081000000000030', name='Purlin cleat')
    cleat['values']['contents'] = cleat['values']['contents'][1:3]
    cleat['values']['contents'][0]['part']['fields'].update(thickness=8)
    bolts = cleat['values']['contents'][1]['junction']
    small = deepcopy(details['Bolt M16'])
    small.update(uid='c081000000000031', name='Bolt M12')
    small['values']['contents'][0]['part']['fastener']['d'] = 12
    bolts['detail'] = small['uid']
    bolts['repeat']['nest']['count'] = 1

    scene = Scene()
    history = History(scene)
    apply(history, 'preset_update', rows=[details['S355 plate'], details['Bolt M16'], details['Fillet weld'], knee, small, cleat])
    for i in range(56):
        x = 3*i
        sweeps = [member({'library': 'en/he200a'}, [[x, 0, 0], [x+2, 0, 0]]),
                  member({'library': 'basic/timber', 'params': {'width': 45, 'height': 486.25}},
                         [[x+1, 0, 0], [x+1, 0, 1]])]
        apply(history, 'create_sweeps', sweeps=sweeps)
        main, joining = list(scene.groups)[-2:]
        apply(history, 'junction_apply', detail=(knee if i < 8 else cleat)['uid'], bindings={
            'main': dict(placement=main.uid, face='flange_outer'),
            'joining': dict(placement=joining.uid, end='start')})
    assert all(r['state'] == 'ok' for r in live.live_rows(scene))
    return scene


def test_garage_junction_schedules():
    from core.bim import junction_schedules
    tables = junction_schedules(garage_junctions())
    assert {r[0]: r[1] for r in tables['fasteners'][1:]} == {16: 32, 12: 96}
    assert {r[-1]: r[4] for r in tables['plates'][1:]} == {'End plate': 8, 'Purlin cleat': 48}
    assert tables['welds'][1][2] == pytest.approx(8.5)
    assert sum(r[-1] for r in tables['holes'][1:]) == 256


def test_garage_ifc_ids(tmp_path):
    ifc = pytest.importorskip('ifcopenshell')
    pytest.importorskip('ifctester')
    scene = garage_junctions()
    path = tmp_path / 'garage.ifc'
    report = save_ifc(scene, path, ids_files=[Path(__file__).parent / 'fixtures' / 'junction-status.ids'])
    assert not report.problems
    check, = report.ids[0]['specifications']
    assert check['status'] and check['passed'] == 56
    model = ifc.open(str(path))
    assert len(model.by_type('IfcMechanicalFastener')) == 128
    assert len(model.by_type('IfcPlate')) == 56
    assert len(model.by_type('IfcVoidingFeature')) == 256


@pytest.mark.parametrize(('x', 'relating'), [(1, 'ATPATH'), (2, 'ATEND')])
def test_wall_path_connection_ends(tmp_path, x, relating):
    ifc = pytest.importorskip('ifcopenshell')
    from built_core.history import History
    from tests.test_live_recipes import apply

    scene = Scene()
    history = History(scene)
    for points, tool in [([[0, 0, 0], [2, 0, 0]], False), ([[x, 0, 0], [x, 1, 0]], True)]:
        apply(history, 'create_sweeps', sweeps=[dict(profile=dict(library='basic/wall'),
              path=dict(points=points))], wall_junctions=tool)
    path = tmp_path / 'walls.ifc'
    save_ifc(scene, path)
    relation, = ifc.open(str(path)).by_type('IfcRelConnectsPathElements')
    assert relation.RelatingConnectionType == relating
    assert relation.RelatedConnectionType == 'ATSTART'
