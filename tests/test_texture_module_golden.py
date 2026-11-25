# SPDX-License-Identifier: GPL-3.0-or-later
"""``core.texture`` must answer as ``core/texture.py`` did at ``58520a91``:
file names for the cache (unsafe characters, digest prefixes stacked by
old saves, long names clipped with their extension), image signatures, the
cache root under each environment setting, content-addressed cache paths,
the cache's stats and clearing, tinted and untinted texture entries (the
tinted image's bytes by hash), and the ``Texture`` record (its fields,
``as_dict``, ``from_dict``, equality and ``repr``). Paths under the test's
temporary folder read as ``<tmp>``. ``tests/fixtures/texture_module_golden.json``;
regenerate only on purpose: ``python tests/test_texture_module_golden.py --write``."""
from __future__ import annotations

import hashlib
import json
import os
import random
import sys
import tempfile
from pathlib import Path

GOLDEN = Path(__file__).parent / "fixtures" / "texture_module_golden.json"

NAMES = ["wood.png", "../../etc/passwd", "0123456789abcdef-wood.png",
         "0123456789abcdef-fedcba9876543210-sumari.png", " .hidden. ", "a" * 90 + ".jpeg",
         "b" * 90, "c" * 70 + ".toolongext", "Ünïcödé ñame.png", "", "   ", "x/y\\z:*?.png",
         "name.with.dots.tga", "d" * 63 + ".png", "e" * 65]


def _png(rgba, w=2, h=2):
    from ingetrazo_render import SurveyRaster
    return SurveyRaster(w, h, bytes(rgba)).material_png(b"")


def answers():
    import core.texture as T
    out = {}
    out["names"] = [T.texture_file_name(n) for n in NAMES]
    heads = [b"\x89PNG\r\n\x1a\nrest", b"\xff\xd8\xff\xe0", b"GIF87a", b"GIF89a", b"BM\x00",
             b"II*\x00", b"MM\x00*", b"RIFF\x00\x00\x00\x00WEBPVP8 ", b"RIFF\x00\x00\x00\x00WAVE",
             b"", b"\x00\x01", b"PK\x03\x04"]
    out["magic"] = [T.is_image_bytes(h) for h in heads]
    out["constants"] = [T.NAME_LIMIT, T.VERTICAL_TOLERANCE, T.COLORIZE_SHIFT, T.COLORIZE_TINT]
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        clean = lambda p: str(p).replace(str(tmp.resolve()), "<tmp>").replace(str(tmp), "<tmp>")
        saved = {k: os.environ.get(k) for k in ("INGETRAZO_TEXTURE_CACHE", "BUILT_SETTINGS_DIR")}
        try:
            os.environ["INGETRAZO_TEXTURE_CACHE"] = str(tmp / "cache")
            out["root"] = clean(T.texture_cache_root())
            out["stats0"] = list(T.texture_cache_stats())
            data = [b"\x89PNG\r\n\x1a\n" + bytes([i]) * 40 for i in range(3)]
            paths = [T.cache_image(d, n, sub) for d, n, sub in
                     zip(data, ["wood.png", "0123456789abcdef-wood.png", "x" * 80 + ".png"],
                         ["skp", "embedded", "skp"])]
            out["cached"] = [clean(p) for p in paths]
            out["cached_again"] = clean(T.cache_image(data[0], paths[0].name, "skp"))
            out["stats1"] = list(T.texture_cache_stats())
            files = []
            for i, b in enumerate([b"\x89PNG\r\n\x1a\nxx", b"notanimage", b"GIF89a.."]):
                p = tmp / f"f{i}.bin"
                p.write_bytes(b)
                files.append(T.is_image_file(p))
            files.append(T.is_image_file(tmp / "missing.png"))
            out["files"] = files
            img = tmp / "tile.png"
            img.write_bytes(_png([200, 100, 50, 255, 10, 20, 30, 255, 90, 90, 90, 0, 255, 255, 255, 255]))
            tex = {"path": str(img), "sw": 0.5, "sh": 0.25}
            tints = []
            for color, mode in (((0.2, 0.4, 0.8), 0), ((1.0, 0.0, 0.0), 1), ((1.5, -0.2, 0.5), 0)):
                t = T.tinted_texture(tex, color, mode)
                again = T.tinted_texture(t, (0.3, 0.3, 0.3), mode)
                tints.append({k: (clean(v) if isinstance(v, str) else v) for k, v in t.items()})
                tints.append({k: (clean(v) if isinstance(v, str) else v) for k, v in again.items()})
                tints.append(hashlib.sha1(Path(t["path"]).read_bytes()).hexdigest())
                u = T.untinted_texture(again)
                tints.append({k: (clean(v) if isinstance(v, str) else v) for k, v in u.items()})
            tints.append(T.tinted_texture({"path": str(tmp / "nope.png")}, (1, 1, 1)) == {"path": str(tmp / "nope.png")})
            tints.append(T.tinted_texture({}, (1, 1, 1)))
            out["tints"] = tints
            out["cutout"] = [T.image_has_cutout(str(img)), T.image_has_cutout(str(img)), T.image_has_cutout(str(tmp / "f1.bin"))]
            out["clear"] = T.clear_texture_cache()
            out["stats2"] = list(T.texture_cache_stats())
            del os.environ["INGETRAZO_TEXTURE_CACHE"]
            os.environ["BUILT_SETTINGS_DIR"] = str(tmp / "profile")
            out["profile_root"] = clean(T.texture_cache_root())
        finally:
            for k, v in saved.items():
                if v is None:
                    os.environ.pop(k, None)
                else:
                    os.environ[k] = v
    t = T.Texture("a.png")
    u = T.Texture.from_dict({"path": "b.png", "sw": "2", "sh": 3})
    out["record"] = [repr(t), t.as_dict(), repr(u), u.as_dict(), t == T.Texture("a.png", 1.0, 1.0),
                     t == u, T.Texture.__name__, T.Texture.__module__,
                     list(T.Texture.__dataclass_fields__)]
    return json.loads(json.dumps(out, default=repr))


def test_texture_module_answers_as_the_python_did():
    assert answers() == json.loads(GOLDEN.read_text())


if __name__ == "__main__":
    if "--write" in sys.argv:
        GOLDEN.write_text(json.dumps(answers(), indent=1))
        print("wrote", GOLDEN)
