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


@pytest.fixture(scope="module")
def composite(tmp_path_factory):
    from tests.composite_fixture import build

    with pytest.MonkeyPatch.context() as env:
        env.setenv("INGETRAZO_RUST_IO", "0")
        paths = make(tmp_path_factory.mktemp("file-io-composite"))
        large = build(tmp_path_factory.mktemp("large-file-io-composite"))
    paths.update({"large-" + key: value for key, value in large.items()})
    return paths


@pytest.mark.parametrize("source", CORPUS + ["leaf", "mid", "host", "large-host", "large-a", "large-b", "large-c", "large-d"], ids=lambda p: Path(p).stem)
@pytest.mark.parametrize("suffix", [".igz", ".built"])
def test_saved_bytes(source, suffix, tmp_path, composite, monkeypatch):
    src = source if isinstance(source, Path) else composite[source]
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
