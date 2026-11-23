"""File bytes at the public save/load entry points, including nested links."""
import itertools
from pathlib import Path
import uuid

import pytest

from core.scene import Scene
from formats import igz
from tests.fixtures.shell.make_composite import make

ROOT = Path(__file__).resolve().parents[1]
CORPUS = sorted((ROOT / "examples").glob("*.igz")) + sorted((ROOT / "tests/fixtures").rglob("*.igz"))


#: The composite at its real size (A holds 20 000 placements) is built only
#: for the ``slow`` cases; the default run uses it scaled down, with the
#: large-document threshold lowered to match, so the same format-3 paths run.
SCALED = 600
SCALED_THRESHOLD = 200


def _composite(tmp_path_factory, name, **build_args):
    from tests.composite_fixture import build
    with pytest.MonkeyPatch.context() as env:
        env.setenv("INGETRAZO_RUST_IO", "0")
        return build(tmp_path_factory.mktemp(name), **build_args)


@pytest.fixture(scope="module")
def composite(tmp_path_factory):
    with pytest.MonkeyPatch.context() as env:
        env.setenv("INGETRAZO_RUST_IO", "0")
        return make(tmp_path_factory.mktemp("file-io-composite"))


@pytest.fixture(scope="module")
def scaled_composite(tmp_path_factory):
    return _composite(tmp_path_factory, "scaled-file-io-composite", big=SCALED, scale=0.1)


@pytest.fixture(scope="module")
def large_composite(tmp_path_factory):
    return _composite(tmp_path_factory, "large-file-io-composite")


_PARTS = ["host", "a", "b", "c", "d"]
SOURCES = (CORPUS + ["leaf", "mid", "host"] + ["scaled-" + k for k in _PARTS]
           + [pytest.param("large-" + k, marks=pytest.mark.slow, id="large-" + k) for k in _PARTS])


@pytest.mark.parametrize("source", SOURCES, ids=lambda p: Path(p).stem)
@pytest.mark.parametrize("suffix", [".igz", ".built"])
def test_saved_bytes(source, suffix, tmp_path, request, monkeypatch):
    monkeypatch.setenv("INGETRAZO_RUST_IO", "1")
    if igz.native_io() is None:
        pytest.skip("native document I/O unavailable")
    if isinstance(source, Path):
        src = source
    elif source.startswith("scaled-"):
        src = request.getfixturevalue("scaled_composite")[source.removeprefix("scaled-")]
        monkeypatch.setattr(igz, "_LARGE_DOCUMENT", SCALED_THRESHOLD)
    elif source.startswith("large-"):
        src = request.getfixturevalue("large_composite")[source.removeprefix("large-")]
    else:
        src = request.getfixturevalue("composite")[source]
    dest = tmp_path / ("saved" + suffix)
    from core import group, layers
    expected = None
    for mode in ("0", "1"):
        # Older files acquire missing identities at open. Give both readers
        # the identical sequence and compare independent loads of the source.
        uids = itertools.count()
        guids = itertools.count()
        def uid():
            return f"uid-{next(uids):05d}"
        monkeypatch.setattr(group, "new_uid", uid)
        monkeypatch.setattr(layers, "new_uid", uid)
        monkeypatch.setattr(uuid, "uuid4", lambda: uuid.UUID(int=next(guids)))
        monkeypatch.setenv("INGETRAZO_RUST_IO", mode)
        scene = Scene()
        igz.load_into(scene, src)
        if isinstance(source, str):
            assert not scene.link_errors
        if source in ("scaled-host", "scaled-a", "large-host", "large-a"):
            assert igz._large_document(scene)       # the format-3 path runs
        igz.save_scene(scene, dest, backup=False)
        if expected is None:
            expected = dest.read_bytes()
        else:
            assert dest.read_bytes() == expected

    out = tmp_path / ("again" + suffix)
    result = []
    for mode in ("0", "1"):
        monkeypatch.setenv("INGETRAZO_RUST_IO", mode)
        reopened = Scene()
        igz.load_into(reopened, dest)
        igz.save_scene(reopened, out, backup=False)
        result.append(out.read_bytes())
    assert result[0] == result[1]


def test_native_zip64_headers(tmp_path, monkeypatch):
    import zipfile

    native = igz.native_io()
    if not hasattr(native, "igz_write_zip"):
        pytest.skip("native archive writer unavailable")
    monkeypatch.setattr(zipfile, "ZIP64_LIMIT", 256)
    rows = [("document.json", True, b"x" * 246, None),
            ("geom/0.bin", False, b"g" * 320, None),
            ("textures/test.png", False, b"png", None)]
    with (tmp_path / "native").open("wb") as dest:
        assert native.igz_write_zip(dest.fileno(), rows, False, 256)
    with zipfile.ZipFile(tmp_path / "reference", "w") as archive:
        for name, compress, data, _ in rows:
            info = zipfile.ZipInfo(name, date_time=igz._ZIP_EPOCH)
            archive.writestr(info, data, compress_type=zipfile.ZIP_DEFLATED if compress else zipfile.ZIP_STORED)
    assert (tmp_path / "native").read_bytes() == (tmp_path / "reference").read_bytes()


def test_overlapping_saves_publish_only_their_own_complete_file(tmp_path):
    path = tmp_path / "drawing.igz"
    path.write_bytes(b"original")
    with pytest.raises(RuntimeError, match="cancel"):
        with igz._atomic_output(path) as first:
            first.write(b"unfinished")
            with igz._atomic_output(path) as second:
                second.write(b"complete")
                assert path.read_bytes() == b"original"
            assert path.read_bytes() == b"complete"
            raise RuntimeError("cancel")
    assert path.read_bytes() == b"complete"
    assert list(tmp_path.iterdir()) == [path]


def test_nested_link_progress_and_cancellation(composite):
    phases = []
    host = composite["host"]
    scene = Scene()
    igz.load_into(scene, host, progress=lambda f, text: phases.append((f, text)))
    assert not scene.link_errors
    assert any("leaf.built" in text for _, text in phases)
    assert all(a[0] <= b[0] for a, b in zip(phases, phases[1:]))
    assert phases[-1][0] == 1

    def stop_at_leaf(fraction, text):
        if "leaf.built" in text:
            raise RuntimeError("cancel linked open")

    with pytest.raises(RuntimeError, match="cancel linked open"):
        igz.load_into(Scene(), host, progress=stop_at_leaf)


def test_native_float_spelling():
    import io
    import json
    import math
    import random
    import struct

    native = igz.native_io()
    if native is None:
        pytest.skip("native document I/O unavailable")
    rng = random.Random(10325)
    values = [-0.0, 0.0, math.ulp(0.0)]
    for power in range(-323, 309):
        x = 10.0 ** power
        for v in (x, math.nextafter(x, 0), math.nextafter(x, math.inf)):
            values.extend([v, -v])
    for _ in range(10000):
        f, = struct.unpack(">d", rng.getrandbits(64).to_bytes(8, "big"))
        if math.isfinite(f):
            values.append(f)
    output = io.BytesIO()
    native.igz_encode_json(values, output.write, igz.Fragment, False)
    assert output.getvalue() == json.dumps(values, separators=(",", ":")).encode()


def test_sheet_view_annotation_and_style_records(tmp_path, monkeypatch):
    from ingetrazo_core import Vec3
    import json
    import zipfile
    from core.composition import Composicion, TextoItem
    from core.dimension import Dimension
    from core.layers import Layer
    from core.materials import Material
    from core.saved_views import SavedView
    from core.textlabel import TextLabel

    scene = Scene()
    extension = {"records": [{"class": "IfcWall", "name": "Mûr"}],
                 "deliveries": [{"sheet": "Elevation", "revision": 2}],
                 "structure": {"groups": [{"xform": list(range(16)),
                                            "faces": [{"extension_field": [1, 2]}]}]}}
    scene.plugin_data["example_extension"] = extension
    scene.layers.append(Layer("Draft", color=(0.123456, 1, 0), lineweight=0.2345,
                              hatch_scale=1.234567, hatch_ground="layer", mortar=True))
    scene.materials["Paint"] = Material("Paint", color=(0.2, 0.4, 1), opacity=0.8)
    scene.saved_views = [SavedView("North", target=(7, 1, -2), hidden_folders=[],
                                  hidden_layers=["Draft"], lens={"roll": 0.25})]
    sheet = Composicion(name="Elevation", border=True, guides_h=[2.0, 4.0])
    sheet.frames = [sheet.default_frame()]
    sheet.texts = [TextoItem(text="Façade")]
    sheet.cajetin = sheet.default_cajetin()
    scene.compositions = [sheet]
    origin, end, offset = Vec3(0, 0, 0), Vec3(1, 0, 0), Vec3(0, 2, 0)
    scene.dimensions = [Dimension(origin, end, offset, layer="Draft")]
    scene.text_labels = [TextLabel(origin, offset, "Note", layer="Draft")]

    source = tmp_path / "sheet.built"
    monkeypatch.setenv("INGETRAZO_RUST_IO", "0")
    igz.save_scene(scene, source, backup=False)
    with zipfile.ZipFile(source) as archive:
        raw = json.loads(archive.read("document.json"))["scene"]
        assert raw["compositions"][0]["texts"][0]["text"] == "Façade"

    def reference_codec(*args, **kwargs):
        pytest.fail("canonical record passed through the reference codec")

    expected = None
    for mode in ("0", "1"):
        monkeypatch.setenv("INGETRAZO_RUST_IO", mode)
        if mode == "1":
            if igz.native_io() is None:
                pytest.skip("native document I/O unavailable")
            for record in (SavedView, Composicion, Layer, Material, TextLabel):
                monkeypatch.setattr(record, "from_dict", reference_codec)
                monkeypatch.setattr(record, "to_dict", reference_codec)
        reopened = Scene()
        igz.load_into(reopened, source)
        assert reopened.compositions[0].texts[0].text == "Façade"
        assert reopened.text_labels[0].text == "Note"
        assert reopened.plugin_data["example_extension"] == extension
        target = tmp_path / "copy.built"
        igz.save_scene(reopened, target, backup=False)
        if expected is None:
            expected = target.read_bytes()
        else:
            assert target.read_bytes() == expected
