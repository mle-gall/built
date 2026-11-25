"""The two filed survey panels from the sheet delivery proof, in paper units."""
from xml.etree import ElementTree as ET

import pytest
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from tests.test_sheets_workspace import host, _sheets  # noqa: F401


def populate(scene):
    from built_core import Vec3, structure
    from built_core.composition import Composicion, MarcoVista, TextoItem, CotaItem
    from built_core.group import Group
    from built_core.history import History
    from core import bim_registry
    registry = bim_registry.seed_defaults()
    registry["structure"] = {"nodes": {
        "shell": {"name": "Envelope", "order": 0, "parent": None},
        "panels": {"name": "Wall panels", "order": 0, "parent": "shell", "recipe": {"class": "IfcWall"}},
    }}
    scene.plugin_data["bim_registry"] = registry
    history = History(scene)
    for name, left, right, height in [("W-01", 0, 4, 3), ("W-02", 4.1, 6, 2.4)]:
        wall = Group(name=name)
        pts = [Vec3(x, y, z) for z in (0, height) for x, y in
               [(left, 0), (right, 0), (right, .25), (left, .25)]]
        for ring in [(3,2,1,0), (4,5,6,7), (0,1,5,4), (1,2,6,5), (2,3,7,6), (3,0,4,7)]:
            wall.mesh.add_face([pts[i] for i in ring])
        scene.groups.append(wall)
        history.execute(structure.file(scene, [wall.uid], "panels"))
        assert structure.node_of(wall) == "panels"
    sheet = Composicion(name="Panel elevation", paper="A4", landscape=True, border=True)
    sheet.frames.append(MarcoVista(x_mm=21, y_mm=39, w_mm=255, h_mm=118, scale_n=30,
                                   view_key="std:front", style="vectorial", border=False, show_title=False))
    sheet.texts.append(TextoItem(x_mm=20, y_mm=15, w_mm=255, size_pt=19, text="SURVEY PANEL ELEVATION"))
    sheet.texts.append(TextoItem(x_mm=20, y_mm=182, w_mm=255, size_pt=10, text="Existing precast panels"))
    sheet.cotas.append(CotaItem(x_mm=48.5, y_mm=156, dx_mm=200, dy_mm=0, scale_n=30, text="Overall <>"))
    scene.compositions.append(sheet)
    scene.version += 1


def check_delivery(path):
    if path.suffix == ".pdf":
        pdf = pytest.importorskip("pymupdf")
        with pdf.open(path) as doc:
            page = doc[0]
            text = page.get_text()
            edges = [tuple((point.x * 25.4 / 72, point.y * 25.4 / 72) for point in item[1:])
                     for drawing in page.get_drawings() for item in drawing["items"] if item[0] == "l"]
    elif path.suffix == ".svg":
        root = ET.parse(path).getroot()
        text = " ".join(root.itertext())
        edges = [tuple((float(line.get("x" + n)), float(line.get("y" + n))) for n in "12")
                 for line in root.findall(".//{*}line")]
    else:
        import ezdxf
        doc = ezdxf.readfile(path)
        audit = doc.audit()
        assert not audit.errors and not audit.fixes
        entities = doc.modelspace()
        text = " ".join(e.dxf.text for e in entities.query("TEXT MTEXT"))
        edges = [tuple((p.x, 210 - p.y) for p in (e.dxf.start, e.dxf.end))
                 for e in entities.query("LINE")]
    assert all(label in text for label in ("SURVEY PANEL ELEVATION", "Overall 6.00 m", "Existing precast panels"))
    for left, top, right in [(48.5, 48, 181.833), (185.167, 68, 248.5)]:
        corners = [(left, top), (right, top), (right, 148), (left, 148)]
        for start, end in zip(corners, corners[1:] + corners[:1]):
            assert any(a == pytest.approx(start, abs=.04) and b == pytest.approx(end, abs=.04)
                       for edge in edges for a, b in (edge, edge[::-1])), (path, start, end)


@pytest.mark.parametrize("kind", ["pdf", "svg", "dxf"])
@pytest.mark.parametrize("reopen", [False, True])
def test_filed_sheet_delivery(host, tmp_path, kind, reopen):
    from formats.igz import save_scene
    scene = host.view.source_viewport.scene
    scene.clear()
    populate(scene)
    model = tmp_path / "panel-elevation.built"
    save_scene(scene, model)
    if reopen:
        assert host.open(model)
    ws = _sheets(host)
    output = tmp_path / ("panels." + kind)
    ws._export_to(kind, str(output))
    assert output.is_file(), ws.status()
    check_delivery(output)


def test_color_by_tag_on_filed_frame_preserves_folder_ids(host, tmp_path):
    from built_render import sheet_frames
    view = host.view.source_viewport
    scene = view.scene
    scene.clear()
    populate(scene)
    before = dict(scene.layer_folder_uids)
    frame = scene.compositions[0].frames[0]
    for kind in ("svg", "dxf"):
        assert getattr(sheet_frames, "export_" + kind)(view, frame, tmp_path / ("frame." + kind)) >= 8
        assert scene.layer_folder_uids == before
