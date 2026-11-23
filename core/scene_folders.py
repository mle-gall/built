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


def folders_of(scene) -> list[str]:
    """The folder names, in order: the ones the document lists, then any a
    scene names that is missing there."""
    out = [f for f in getattr(scene, "scene_folders", None) or ()
           if isinstance(f, str) and f]
    out = list(dict.fromkeys(out))
    for v in getattr(scene, "saved_views", None) or ():
        if v.folder and v.folder not in out:
            out.append(v.folder)
    return out


def folder_parent(scene, name: str) -> str | None:
    """The folder ``name`` sits in, or ``None`` at the top. A parent that
    is gone, or one that would close a loop, counts as the top."""
    parents = getattr(scene, "scene_folder_parents", None) or {}
    known = set(folders_of(scene))
    parent = parents.get(name)
    if not parent or parent not in known or parent == name:
        return None
    seen = {name}
    up = parent
    while up is not None:
        if up in seen:
            return None
        seen.add(up)
        up = parents.get(up) if parents.get(up) in known else None
    return parent


def folder_of(scene, view) -> str | None:
    """The folder ``view`` is shown in (``None``: at the top)."""
    return view.folder if view.folder in folders_of(scene) else None


def subfolders(scene, name: str | None, deep: bool = False) -> list[str]:
    """The folders directly in ``name`` (``None``: the top) — or, ``deep``,
    at any depth."""
    out = [f for f in folders_of(scene) if folder_parent(scene, f) == name]
    if deep:
        for f in list(out):
            out += subfolders(scene, f, deep=True)
    return out


def scenes_in(scene, name: str | None) -> list:
    """The scenes directly in ``name`` (``None``: the top), in order."""
    return [v for v in scene.saved_views if folder_of(scene, v) == name]


def tree_order(scene) -> list:
    """Every scene, read from top to bottom: in each level the subfolders
    first (with all they hold), then its scenes."""
    out: list = []

    def level(name) -> None:
        for f in subfolders(scene, name):
            level(f)
        out.extend(scenes_in(scene, name))
    level(None)
    return out


def sync_order(scene) -> None:
    """``scene.saved_views`` in the tree's order (the same list object)."""
    order = tree_order(scene)
    if [id(v) for v in order] != [id(v) for v in scene.saved_views]:
        scene.saved_views[:] = order


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


def state(scene) -> tuple:
    """Everything a folder change touches, to put back on undo."""
    return (list(scene.saved_views),
            [(v, v.folder) for v in scene.saved_views],
            list(getattr(scene, "scene_folders", None) or ()),
            dict(getattr(scene, "scene_folder_parents", None) or {}))


def signature(scene) -> tuple:
    """What the tree shows, comparable: whether a change changed anything."""
    return (tuple((id(v), v.folder) for v in scene.saved_views),
            tuple(getattr(scene, "scene_folders", None) or ()),
            tuple(sorted((getattr(scene, "scene_folder_parents", None)
                          or {}).items())))


def restore(scene, saved: tuple) -> None:
    views, folders_of_views, folders, parents = saved
    scene.saved_views[:] = views
    for v, f in folders_of_views:
        v.folder = f
    scene.scene_folders = list(folders)
    scene.scene_folder_parents = dict(parents)
