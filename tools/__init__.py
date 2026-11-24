# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""IngeTrazo built-in tools (line, rectangle, push/pull, select, ...)."""

from functools import wraps


def _plugin_method(native):
    """Expose a Python function to signal libraries that inspect bound slot code."""
    @wraps(native)
    def method(*args, **kwargs):
        return native(*args, **kwargs)
    return method
