# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Select's viewport actions and compatibility paths for materialised meshes.
Store-backed meshes grow picks through native incidence. The native viewport
box-selects placements from the renderer's packed prototype arrays.
"""
from __future__ import annotations


from ingetrazo_core import (box_records, box_placements, box_chunk_group, chunk_bounds as _local_bounds, box_from_pack as _box_groups_packed,
                            segment_overlaps_box as _seg_rect_overlap)

from core.dimension import Dimension
from core.textlabel import TextLabel
from core.group import Group
from core.mesh import Edge, Face
from core.guide import is_guide
from core.section import SectionPlane
from core.history import (
    CompoundCommand,
    DeleteDimensionsCommand,
    DeleteGeoPathsCommand,
    DeleteGroupCommand,
    DeleteGuidesCommand,
    DeleteSectionPlanesCommand,
    DeleteTextLabelsCommand,
    SetActiveSectionCommand,
    EditTextLabelCommand,
    EraseSelectionCommand,
)
from georef.geopath import GeoPath
from ingetrazo_core import register_tool_functions

register_tool_functions(globals())


#: A repeated plain click within this many pixels takes the next candidate
#: (the machine's ``CYCLE_PX``).
CYCLE_PX = 4.0


def _edit_text(viewport, entity) -> None:
    # As usual, double-clicking a leader text edits its text.
    from core.i18n import tr
    from ingetrazo_render import chrome_forms

    def edit(text):
        if text is not None and text.strip() and text.strip() != entity.text:
            viewport.history.execute(
                EditTextLabelCommand(entity, text.strip()))
        viewport.update()
    chrome_forms.ask_text(viewport.window(), tr("Text"), tr("Label text:"),
                          entity.text, done=edit, multiline=True)


def _edit_dimension(viewport, entity) -> None:
    # Double-clicking the dimension text edits it; "<>" stands for the
    # measured value, and an empty text (or a bare "<>") goes back to the
    # automatic value.
    from core.i18n import tr
    from core.history import EditDimensionTextCommand
    fmt = getattr(viewport, "_format_dim_value", None)
    style = getattr(viewport.scene, "dimension_style", {}) or {}
    measured = (fmt(entity.value(), style) if fmt is not None
                else entity.label())
    current = entity.text if entity.text else measured
    from ingetrazo_render import chrome_forms

    def retext(text):
        if text is not None:
            new = text.strip()
            if new in ("", "<>", measured):
                new = None
            if new != entity.text:
                viewport.history.execute(
                    EditDimensionTextCommand(entity, new))
        viewport.update()
    chrome_forms.ask_text(viewport.window(), tr("Dimension"),
                          tr("Dimension text (<> = measured value):"),
                          current, done=retext)


#: The double-click verbs the host still owns (the machine names them).
HOST_VERBS = {"toggle_section": _toggle_section, "edit_text": _edit_text,
              "edit_dimension": _edit_dimension}
