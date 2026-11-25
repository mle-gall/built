# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
""".skp document import.

The Rust reader builds definition-local geometry and placement records on the
document worker. The classic model provider is loaded only for MFC files.
`parse_skp` also exposes the detached payload to scripts. Applying it is a
separate document transaction, so a failed read cannot partially change a scene.
"""
from __future__ import annotations

from ingetrazo_core import ModelImportBackend, NeedsConverter


#: Reader order for import and capability queries.
_BACKENDS: list = [ModelImportBackend()]


from ingetrazo_core import (
    skp_apply as apply_payload, skp_apply_folders as _import_tag_folders,
    skp_soften_bulk as _soften_bulk_edges, skp_load as load_skp,
    skp_backends_status as backends_status, skp_can_handle as can_handle,
    skp_detect_format as detect_format, skp_dispatch_parse as parse_skp,
)
