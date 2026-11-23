# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""A terrain profile item's ground (ingetrazo_core sheets::profile) is
what ``ComposerWindow.profile_for`` sampled with ``georef.profile
.sample_profile``: the same stations, the same points, the same
elevations bit for bit off the same DEM fields (bilinear in float32), the
same messages while there is nothing to plot. Synthetic tiles, no network:
the sampler's ``ensure_area`` only records what it was asked for."""
from __future__ import annotations

import os

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

import numpy as np  # noqa: E402
import pytest  # noqa: E402
from PySide6.QtWidgets import QApplication  # noqa: E402

import ingetrazo_core as NATIVE  # noqa: E402

_app = QApplication.instance() or QApplication([])


class _Dem:
    """georef.dem.DEMSampler without the fetcher: its fields are given."""

    def __init__(self, datum, zoom=13, missing=()):
        from georef.dem import DEMSampler
        self._real = DEMSampler(datum, zoom=zoom)
        self.datum, self.zoom = datum, zoom
        self._fields = self._real._fields
        self._missing = set(missing)
        self.asked = []

    def fill(self, lat_s, lon_w, lat_n, lon_e):
        from georef.tiles import tiles_covering
        rng = np.random.default_rng(7)
        for tx, ty in tiles_covering(lat_s, lon_w, lat_n, lon_e, self.zoom):
            if (tx, ty) not in self._missing:
                base = rng.uniform(2400.0, 2600.0)
                gy, gx = np.mgrid[0:256, 0:256]
                self._fields[(tx, ty, self.zoom)] = (base + 0.37 * gx - 0.21 * gy
                                                     + rng.normal(0, 3.0, (256, 256))).astype(np.float32)

    def ensure_area(self, *bbox):
        self.asked.append(bbox)
        self.fill(*bbox)

    def elevation_at_local(self, p):
        lat, lon, _ = self.datum.local_to_geodetic(p)
        from georef.dem import bilinear, pixel_in_tile
        tx, ty, px, py = pixel_in_tile(lat, lon, self.zoom)
        field = self._fields.get((tx, ty, self.zoom))
        return None if field is None else bilinear(field, px, py)


def _scene(paths, datum=True):
    from ingetrazo_core import Vec3 as V
    from core.scene import Scene
    from georef.datum import SceneDatum as Datum
    from georef.geopath import GeoPath
    s = Scene()
    s.georef = Datum(lat=-16.4, lon=-71.53, alt=2330.0, north=12.0) if datum else None
    s.geo_paths = [GeoPath(points=[V(x, y, 0.0) for x, y in pts], name=name, closed=closed)
                   for name, pts, closed in paths]
    return s


def _comp(items):
    from core.composition import Composicion, PerfilTerreno
    c = Composicion(name="Perfiles")
    for kw in items:
        c.perfiles.append(PerfilTerreno(**kw))
    return c


PATHS = [("Canal", [(0, 0), (850, 120), (1900, -300), (2600, 410)], False),
         ("", [(10, 10), (-400, 900)], False),
         ("Loop", [(0, 0), (300, 0), (300, 300)], True),
         ("Dot", [(5, 5)], False)]


@pytest.mark.parametrize("index,spacing", [(0, 0.0), (0, 25.0), (1, 0.0), (1, 7.5), (2, 0.0), (2, 40.0)])
def test_the_samples_are_the_composer_s(index, spacing):
    from georef.profile import sample_profile
    scene = _scene(PATHS)
    comp = _comp([dict(path_index=index, spacing_m=spacing)])
    dem = _Dem(scene.georef)
    got = NATIVE.sheet_terrain_profiles(comp, scene, dem)
    prof, name, message = got[comp.perfiles[0]._doc_key]
    ref = _Dem(scene.georef)
    want = sample_profile(scene.geo_paths[index].profile_points(), ref, spacing=spacing or None)
    assert dem.asked == ref.asked                                     # the same area asked for
    assert prof["length"] == want.length and prof["complete"] == want.complete
    assert [(s, x, y, e) for s, x, y, e in prof["samples"]] == [
        (w.station, w.x, w.y, w.elevation) for w in want.samples]
    assert name == (scene.geo_paths[index].name or f"Path {index + 1}") and message is None


def test_what_the_item_says_before_there_is_a_profile():
    from georef.profile import sample_profile
    comp = _comp([dict(path_index=0)])
    key = comp.perfiles[0]._doc_key
    no_paths = NATIVE.sheet_terrain_profiles(comp, _scene([]), _Dem(None))[key]
    assert no_paths == (None, "", "Trace a path with the Path tool (T) first.")
    gone = _comp([dict(path_index=7)])
    assert NATIVE.sheet_terrain_profiles(gone, _scene(PATHS), _Dem(None))[gone.perfiles[0]._doc_key] == (
        None, "", "Path 8 no longer exists.")
    no_datum = _scene(PATHS, datum=False)
    assert NATIVE.sheet_terrain_profiles(comp, no_datum, _Dem(None))[key] == (
        None, "Canal", "Set a base map location first (Tray ▸ Base map).")
    dot = _comp([dict(path_index=3)])
    assert NATIVE.sheet_terrain_profiles(dot, _scene(PATHS), _Dem(None))[dot.perfiles[0]._doc_key][2] == (
        "The path needs two points.")
    # Tiles still loading: some samples missing, then none at all.
    scene = _scene(PATHS)
    from georef.tiles import tiles_covering
    lats, lons = zip(*(scene.georef.local_to_geodetic(p)[:2] for p in scene.geo_paths[0].profile_points()))
    tiles = tiles_covering(min(lats), min(lons), max(lats), max(lons), 15)
    assert len(tiles) > 2
    from georef.dem import pixel_in_tile
    first = pixel_in_tile(lats[0], lons[0], 15)[:2]           # the tile under the path's start
    some = _Dem(scene.georef, zoom=15, missing=[first])
    prof, _name, message = NATIVE.sheet_terrain_profiles(comp, scene, some)[key]
    want = sample_profile(scene.geo_paths[0].profile_points(), _Dem(scene.georef, zoom=15, missing=[first]))
    assert prof["complete"] is False and want.complete is False and message == "(loading DEM…)"
    assert [e for *_, e in prof["samples"]] == [w.elevation for w in want.samples]
    none = _Dem(scene.georef, zoom=15, missing=tiles)
    assert NATIVE.sheet_terrain_profiles(comp, scene, none)[key][2] == "Loading terrain…"
