# SPDX-License-Identifier: GPL-3.0-or-later
"""The Built project manifest (docs/builtproject-format.md).

The file I/O, the envelope checks, conversion and merging are Rust
(``ingetrazo_core.project_*``); the registry's contents stay Python's
(``core.bim_registry.validate``), and link filters the Navigator grammar's.
"""
import json
from pathlib import Path

SUFFIX = '.builtproject'
LEGACY_SUFFIX = '.igzproj'
SUFFIXES = (SUFFIX, LEGACY_SUFFIX)


from ingetrazo_core import (
    is_project, project_check as _check, project_open_path as open_path,
    project_load as load, project_peek as peek, project_save as save, project_create as create,
)
