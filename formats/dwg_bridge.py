# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""LibreDWG converter discovery, execution and CAD repair."""
from __future__ import annotations

import shutil
import sys
import tempfile

from ingetrazo_render import DwgConverter
from ingetrazo_core import DwgBridgeError

from ingetrazo_core.paths import app_root

_VENDOR_BIN = app_root() / "vendor" / "libredwg" / "bin"
#: Development-machine courtesy: the sibling IngeCAD checkout's bundle.
_SIBLING_BIN = (app_root().parent.parent / "ingecad" / "vendor"
                / "libredwg" / "bin")
_TIMEOUT = 300  # seconds; big real-world DWGs convert in well under this


from ingetrazo_core import (
    dwg_native_executable as _is_native_executable, dwg_find_tool as _find_tool,
    dwg_find_converter as find_dwg2dxf, dwg_supported as have_dwg_support,
    dwg_check_output as _check_output, dwg_run as _run, dwg_command as _converter_command,
    dwg_strip_null as _strip_null_handles, dwg_dedupe as _dedupe_handles,
    dwg_repair as _repair_dxf, dwg_new_output as _new_output,
    dwg_convert as dwg_to_dxf, dwg_discard as discard_temp_dxf,
)
