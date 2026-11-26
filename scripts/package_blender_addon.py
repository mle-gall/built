#!/usr/bin/env python3
"""Package the Built Link add-on for Blender as an installable zip.

    .venv/bin/python scripts/package_blender_addon.py [dest.zip]

Writes ``built_link.zip`` (default: build/built_link.zip) with the same
writer the Render drawer's "Install Blender add-on…" uses. In Blender:
Edit › Preferences › Add-ons › Install from Disk…
"""
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def main(argv):
    from built_core import render_blender as rb
    dest = Path(argv[1]) if len(argv) > 1 else ROOT / "build" / "built_link.zip"
    path = rb.write_addon_zip(dest, ROOT / "integrations" / "blender" / "built_link")
    print(path)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
