# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Flatpak must ship the sheet exporter and forward its command-line arguments."""
from pathlib import Path

FLATPAK = Path(__file__).resolve().parents[1] / "packaging" / "flatpak"


def test_native_sheet_export_is_packaged():
    manifest = (FLATPAK / "com.built.Built.yml").read_text()
    assert "rust/built_app/target/release/built ${FLATPAK_DEST}/bin/built" in manifest
    sources = next(line.split() for line in manifest.splitlines() if "- cp -a core " in line)
    assert {"formats", "views"} <= set(sources)
    launcher = (FLATPAK / "flatpak-launcher.sh").read_text()
    assert 'exec /app/bin/built "$@"' in launcher.splitlines()
