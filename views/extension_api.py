# SPDX-License-Identifier: GPL-3.0-or-later
"""The public plugin API supplied to setup(app), also available to standalone scripts."""
from sys import modules as _modules

from built_render import install_interaction_module as _install

_install(_modules[__name__])
del _install, _modules
