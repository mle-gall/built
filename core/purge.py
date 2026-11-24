# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Purge unused: which tags and materials the document uses, and what
carries a tag. The survey runs in Rust (rust/ingetrazo_core/src/doc/purge.rs,
docs/commands-port.md); these are its existing names."""
from __future__ import annotations

from ingetrazo_core import (LayerCountSteps, purge_annotations as _annotations,
                            purge_count_edges as _count_edges,
                            purge_document_groups as _document_groups,
                            purge_iter_groups as iter_groups, purge_iter_meshes as iter_meshes,
                            purge_layer_carriers as layer_carriers,
                            purge_layers_held_by_scenes as layers_held_by_scenes,
                            purge_unused_layers as unused_layers,
                            purge_unused_materials as unused_materials,
                            purge_used_layers as used_layers,
                            purge_used_materials as used_materials)

__all__ = ["iter_groups", "iter_meshes", "layer_carriers", "layer_count_steps", "layer_counts",
           "layers_held_by_scenes", "unused_layers", "unused_materials", "used_layers",
           "used_materials", "_annotations", "_count_edges", "_document_groups"]


def layer_count_steps(scene, *, photo=True, history=None):
    """None between meshes and group batches, then one ``{tag: count}``."""
    return LayerCountSteps(scene, photo, history)


def layer_counts(scene, *, photo: bool = True) -> dict[str, int]:
    """Every tag carrier counted, as the Layers panel counts."""
    return next(c for c in layer_count_steps(scene, photo=photo) if c is not None)
