# SPDX-License-Identifier: GPL-3.0-or-later
"""Compatibility import for extensions written against the Python Line tool.
The tool is a Rust state machine now (rust/ingetrazo_core/src/tools/line.rs)."""
from tools.rust_tools import LineTool  # noqa: F401
