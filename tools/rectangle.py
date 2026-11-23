# SPDX-License-Identifier: GPL-3.0-or-later
"""Compatibility import for extensions written against the Python Rectangle.
The tool is a Rust state machine now (rust/ingetrazo_core/src/tools/rectangle.rs)."""
from tools.rust_tools import RectangleTool  # noqa: F401
