"""Built document identity and precision at the Qt/model boundary."""

from ingetrazo_core import (
    is_built, built_displacement as displacement_mm,
    built_position as position,
    built_placement as placement,
    built_placement_values as placement_values,
    built_point_array as point_array,
    built_compact_values as compact_values,
    built_export_placements as export_placements,
)

BUILT_SUFFIX = ".built"
DOCUMENT_SUFFIXES = (BUILT_SUFFIX, ".igz")
DOCUMENT_FILTER = f"Documents (*{BUILT_SUFFIX} *.igz)"
BUILT_FILTER = f"Built document (*{BUILT_SUFFIX})"
WARNING_MM = 0.0254
