# SPDX-License-Identifier: GPL-3.0-or-later
"""CAD drawing import and the exception presented by the import dialog."""
from ingetrazo_core import (
    dxf_load as load_dxf,
    dxf_open_document as open_document,
    dxf_prepare_import as prepare_import,
    dxf_read_unit_scale as read_unit_scale,
    dxf_suggest_unit_scale as suggest_unit_scale,
)
from formats.recenter import RECENTER_BEYOND_M, survey_offset


class NothingToImport(ValueError):
    """The file reads, but its model space has nothing to build. ``skipped``:
    the entity kinds left out, with how many of each."""

    def __init__(self, skipped=None) -> None:
        super().__init__("no importable geometry (lines, polylines, arcs, "
                         "circles, ellipses, splines or faces) in the "
                         "model space")
        self.skipped = dict(skipped or {})
