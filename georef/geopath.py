# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
from functools import partialmethod
from core.doc_model import Record, record_fields
from ingetrazo_core import (
    geo_path_init, path_segments, path_length, path_area, path_surface_area, profile_points, geo_record_dict, geo_path_from,
)


@record_fields("points", "closed", "name", "surface")
class GeoPath(Record):
    __init__ = partialmethod(geo_path_init)
    segments = partialmethod(path_segments)
    length = partialmethod(path_length)
    perimeter = partialmethod(path_length)
    area = partialmethod(path_area)
    surface_area = partialmethod(path_surface_area)
    profile_points = partialmethod(profile_points)
    to_dict = partialmethod(geo_record_dict)
    from_dict = classmethod(geo_path_from)
