# SPDX-License-Identifier: GPL-3.0-or-later
"""Document views and their native interaction modules."""
from functools import wraps as _wraps
from ingetrazo_core import install_view_modules as _install_view_modules

_install_view_modules(__path__)


def _host_method(native):
    """Bind the instance for hosts that call an unbound plugin method by keyword."""
    @_wraps(native)
    def method(self, *args, **kwargs):
        return native(self, *args, **kwargs)
    return method


def _plugin_method(native):
    """Give plugin signal libraries a real Python function to inspect."""
    @_wraps(native)
    def method(*args, **kwargs):
        return native(*args, **kwargs)
    return method
