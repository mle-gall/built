# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Folders of the Scenes tab (#1158) — the way the Layers tab files layers.

A folder is a name; ``scene.scene_folders`` lists them in order, empty ones
included, and ``scene.scene_folder_parents`` says which folder each one
sits in (absent: at the top), so folders nest as deep as wanted. A scene
names its folder in ``SavedView.folder`` (``None``: at the top; a folder
the document does not have counts as the top, so a scene never drops out
of sight).

Within a folder the subfolders come first, then the scenes, as in Layers.
``scene.saved_views`` stays THE order of the scenes — the scene tabs, the
sheets' list and every menu read it — and :func:`sync_order` keeps it equal
to the tree read from top to bottom, so the order is the same everywhere.

Nothing here is undoable by itself: :class:`core.history.SceneTreeCommand`
runs these helpers and keeps the undo.
"""
from __future__ import annotations

from ingetrazo_core import scene_tree as _tree

FolderEdit = _tree.FolderEdit


folders_of = _tree.folders_of
folder_parent = _tree.folder_parent
folder_of = _tree.folder_of
subfolders = _tree.subfolders
scenes_in = _tree.scenes_in
tree_order = _tree.tree_order
sync_order = _tree.sync_order


def unique_name(taken, base: str) -> str:
    taken = set(taken)
    if base not in taken:
        return base
    n = 2
    while f"{base} {n}" in taken:
        n += 1
    return f"{base} {n}"


# Commands, plugins and scripts use the same ordered tree edits.
add_folder = _tree.add_folder
rename_folder = _tree.rename_folder
delete_folder = _tree.delete_folder
move_folder = _tree.move_folder


def move_rows(scene, views, folders, folder, before=None):
    _tree.move_rows(scene, list(views), list(folders), folder, before)


state = _tree.state
signature = _tree.signature
restore = _tree.restore
