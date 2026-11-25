# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Render the Library's bundled component thumbnails without a model window."""
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))


def generate(components: Path, output: Path):
    from ingetrazo_core.document_insert import component_from_scene
    from core.scene import Scene
    from ingetrazo_core.formats import igz
    from ingetrazo_render import component_thumbnail

    output.mkdir(parents=True, exist_ok=True)
    for entry in json.loads((components / "components.json").read_text(encoding="utf-8")):
        key = entry["key"]
        scene = Scene()
        igz.load_into(scene, components / f"{key}.igz")
        definition = component_from_scene(scene, entry["name"])
        if definition is None:
            raise ValueError(f"{key} contains no geometry")
        if not component_thumbnail(definition, 320).save(str(output / f"{key}.png")):
            raise OSError(f"Could not write the thumbnail for {key}")


if __name__ == "__main__":
    from scripts.isolated_launch import isolate_script
    isolate_script()
    components = ROOT / "resources" / "components"
    generate(components, components / "thumbs")
