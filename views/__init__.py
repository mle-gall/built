# SPDX-License-Identifier: GPL-3.0-or-later
"""Document views and their native interaction modules."""
from ingetrazo_core import install_view_modules as _install_view_modules

_install_view_modules(__path__)


def _host_method(native):
    """Bind the instance for hosts that call an unbound plugin method by keyword."""
    from functools import wraps

    @wraps(native)
    def method(self, *args, **kwargs):
        return native(self, *args, **kwargs)
    return method
