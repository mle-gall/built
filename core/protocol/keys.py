# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Public protocol keys."""
from ingetrazo_core.command_protocol import (
    mesh_key,
    known_mesh,
    resolve_mesh,
    kind_of,
    store_of,
    element,
    elements,
    to_json_key,
    from_json_key,
)

KINDS = ("vertex", "edge", "face")
