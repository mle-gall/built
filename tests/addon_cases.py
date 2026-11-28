"""Portable add-on fixtures shared by the recording scenes and protocol tests."""
from copy import deepcopy
from functools import lru_cache
from pathlib import Path
from tempfile import TemporaryDirectory

from scripts.build_addons import build

ADDONS = Path(__file__).resolve().parents[1] / "addons"


@lru_cache
def _rows(pack):
    from built_core import command_protocol
    with TemporaryDirectory() as folder:
        path = build(ADDONS / pack, Path(folder))
        return command_protocol.pack_read(str(path))["rows"]


def rows(pack):
    return deepcopy(_rows(pack))


def profile(key):
    region = key.partition("/")[0]
    if region in ("en", "aisc"):
        return next(row["values"]["profile"] for row in rows("steel-profiles-" + region)
                    if row.get("profile_size") and row["values"]["profile"]["library"] == key)
    pack = {"basic/timber": "timber-joints", "basic/stud": "timber-joints", "basic/duct": "mep-penetrations"}.get(key)
    if pack:
        return next(row["values"]["profile"] for row in rows(pack)
                    if row.get("profile_size") and row["values"]["profile"]["library"] == key)
    return {"library": key}


def install(win, pack, folder):
    from built_render import wgpu_library
    path = build(ADDONS / pack, folder)
    result = wgpu_library.import_pack(win, str(path), "mine")
    assert result["needs"] == 0
    return result
