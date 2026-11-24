# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""IngeTrazo views: the window's panels and the Python glue around the
native chrome."""
import sys as _sys

# Deprecated names that extensions written before the chrome moved to Rust
# still import (``from views.chrome_forms import Field, Form, show``,
# ``views.chrome_dialogs.Job``): the native modules under their old names.
# New code imports ``ingetrazo_render.chrome_forms`` / ``chrome_dialogs``
# (docs/plugins.md, "Forms, questions and jobs").
for _old, _new in (("chrome_forms", "chrome_forms"), ("chrome_dialogs", "chrome_dialogs")):
    if f"views.{_old}" not in _sys.modules:
        try:
            _module = getattr(__import__("ingetrazo_render"), _new)
        except (ImportError, AttributeError):  # a tool without the native crate
            continue
        _sys.modules[f"views.{_old}"] = _module
        globals()[_old] = _module
del _old, _new
