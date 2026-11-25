# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""File-format import / export: OBJ, COLLADA, glTF, STL, 3MF, IFC, native .igz.

Named ``formats`` rather than ``io`` to avoid shadowing the Python standard
library ``io`` module.
"""

import sys as _sys
from ingetrazo_core import install_codec_modules as _install_codec_modules

_install_codec_modules(_sys.modules[__name__])
