# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""LibreDWG converter discovery, execution and CAD repair."""
from __future__ import annotations

import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Optional

from ingetrazo_render import DwgConverter

from core.paths import app_root

_VENDOR_BIN = app_root() / "vendor" / "libredwg" / "bin"
#: Development-machine courtesy: the sibling IngeCAD checkout's bundle.
_SIBLING_BIN = (app_root().parent.parent / "ingecad" / "vendor"
                / "libredwg" / "bin")
_TIMEOUT = 300  # seconds; big real-world DWGs convert in well under this


class DwgBridgeError(Exception):
    """A DWG conversion failed or no converter is available.

    ``kind`` tells the caller what to say to the user: ``"unavailable"`` (no
    converter), ``"failed"``, ``"timeout"`` or ``"cancelled"``."""

    def __init__(self, message: str, kind: str = "failed") -> None:
        super().__init__(message)
        self.kind = kind


from ingetrazo_core import (
    dwg_native_executable as _is_native_executable, dwg_find_tool as _find_tool,
    dwg_find_converter as find_dwg2dxf, dwg_supported as have_dwg_support,
    dwg_check_output as _check_output, dwg_run as _run, dwg_command as _converter_command,
    dwg_strip_null as _strip_null_handles, dwg_dedupe as _dedupe_handles,
    dwg_repair as _repair_dxf, dwg_new_output as _new_output,
    dwg_convert as dwg_to_dxf, dwg_discard as discard_temp_dxf,
)
