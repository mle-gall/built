# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Native allocator policy at process startup and document teardown."""
from pathlib import Path
import platform

from ingetrazo_core import trim_heap


def test_native_launcher_caps_glibc_arenas_before_dispatch():
    root = Path(__file__).resolve().parents[1]
    src = (root / "rust/built_app/src/launcher.rs").read_text()
    startup = src.split("pub fn register_launcher()", 1)[1]
    assert startup.index("limit_allocator_arenas();") < startup.index("let args: Vec<_>")


def test_trimming_is_available_on_glibc():
    assert trim_heap() is (platform.libc_ver()[0] == "glibc")
