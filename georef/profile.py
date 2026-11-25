# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
from dataclasses import dataclass, field
from functools import partialmethod
from ingetrazo_core import (
    selected_geopath, sample_profile, point_at_station, polyline_length, profile_to_csv,
    elevations, min_elevation, max_elevation, total_gain, slopes,
)

@dataclass
class ProfileSample:
    """One sampled point along the trace."""
    station: float                  # horizontal distance from the start (m)
    x: float                        # local scene metres
    y: float
    elevation: float | None         # terrain elevation (m), None if not loaded


@dataclass
class Profile:
    """A sampled terrain profile along a polyline."""
    samples: list[ProfileSample] = field(default_factory=list)
    length: float = 0.0
    complete: bool = True           # every sample has an elevation

    elevations = partialmethod(elevations)
    min_elevation = partialmethod(min_elevation)
    max_elevation = partialmethod(max_elevation)
    total_gain = partialmethod(total_gain)
    slopes = partialmethod(slopes)


VERTICAL_NOTES = {
    "odm": ("photogrammetric survey (ODM); WGS84 ellipsoidal heights unless "
            "the processing used ground control points"),
    "dem": "global DEM (SRTM-derived, orthometric)",
    "local": "local reference: heights are relative, not absolute",
}
