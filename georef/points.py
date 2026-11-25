# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
from functools import partialmethod
from core.doc_model import Record, record_fields
from ingetrazo_core import (
    geo_point_init, geo_record_dict, geo_point_from, parse_points_csv, points_from_rows, datum_for_rows,
)


@record_fields("position", "name", "desc")
class GeoPoint(Record):
    __init__ = partialmethod(geo_point_init)
    to_dict = partialmethod(geo_record_dict)
    from_dict = classmethod(geo_point_from)
