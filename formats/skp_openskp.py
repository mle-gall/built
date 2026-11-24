# SPDX-License-Identifier: GPL-3.0-or-later
"""Compatibility import for callers of the former .skp adapter."""
import sys
from formats import skp_reader

sys.modules[__name__] = skp_reader
