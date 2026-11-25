"""Exercise the installed command-line entry point with an on-disk document."""
from built_core.composition import Composicion, MarcoVista
from built_core.scene import Scene
from formats.igz import save_scene
from tests.launcher_binary import run


def test_pdf_export_opens_document_and_writes_all_sheets(tmp_path):
    scene = Scene()
    scene.compositions.extend([Composicion(name="Plan"), Composicion(name="Details")])
    scene.compositions[0].frames.append(MarcoVista(x_mm=10, y_mm=10, w_mm=30,
                                                 h_mm=20, scale_n=100, view_key="std:iso",
                                                 style="sombreado"))
    path = tmp_path / "two sheets.igz"
    save_scene(scene, path)
    result = run("-i", str(path), "-p")
    assert result.returncode == 0, result.stderr
    import pytest
    pymupdf = pytest.importorskip("pymupdf")
    with pymupdf.open(path.with_suffix(".pdf")) as doc:
        assert len(doc) == 2
        assert doc[0].get_images()
