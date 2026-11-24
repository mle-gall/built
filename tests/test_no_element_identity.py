# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""No mesh element is compared with ``is`` in production code.

A store-backed mesh (``core/store_mesh.py``) hands out a new view object on
every read: two views of one face are ``==`` but never ``is``. A Python
element compares by identity anyway, so ``==`` is right for both kinds and
``is`` is right for neither once views are about (it silently answers
False on the store: see ``tests/test_store_read_paths.py`` for what that
broke). This guard parses the code and flags ``a is b`` / ``a is not b``
whose two sides are named like elements (``face``, ``e0``, ``v``,
``self.base_face``, ``loop[i]``...). Names are a heuristic, so the
deliberate exceptions are listed below with the reason each keeps
identity, by enclosing function; a new hit is either a bug or a new entry
with its reason."""
from __future__ import annotations

import ast
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# Directories scanned whole, and the format modules that read or write the
# scene's meshes (the rest of formats/ parses files into fresh Python data).
_DIRS = ("core", "tools", "views", "plugins", "examples")
_FORMAT_FILES = ("fuse.py", "meshexport.py", "obj.py", "stl.py", "gltf.py",
                 "glb.py", "dae.py", "dxf_out.py", "igz.py", "skp.py",
                 "recenter.py")

# The leaf name of a side (``self.base_face`` -> ``base_face``, ``lp[i]`` ->
# ``lp``, ``e.other(v)`` -> ``other``) that reads as a mesh element.
_ELEMENT = re.compile(
    r"^(?:[a-hpqsvwxy]|[efgv]\d|v[ab]|"
    r"\w*face\w*|\w*edge\w*|\w*vert\w*|\w*vertex\w*|"
    r"other|start|cur|nxt|prev|last|reached|first|second|"
    r"loop|lp|ring|verts|chain|targets|out|hit|fp|exclude|"
    r"anchor|filled|seed|mid|holder|keep|dup|small|target|"
    r"[a-z]_(?:face|edge|vert))$")

# (path, enclosing function) -> why identity is meant there.
_KERNEL = ("kernel: runs on Python objects only (a StoreMesh materialises "
           "first through _on_python; MaterialisedMesh resolves the views "
           "it is handed through _resolving)")
_ALLOWED = {
    # The Mesh kernel itself, mutation and its helpers.
    **{("core/mesh.py", fn): _KERNEL for fn in (
        "registers", "find_edge", "add_edge", "add_edges_welded", "_rekey",
        "_insert_into_face_loops")},
    # Not elements: names that look like them.
    **{("core/triangulate.py", fn): "ear-clipping list nodes" for fn in (
        "_is_ear", "_cure_local_intersections", "_split_earcut",
        "_get_leftmost", "_middle_inside", "_intersects_polygon")},
    ("core/topology.py", "__call__"): "id()-keyed memo of a QVector3D",
    ("core/history_commands.py", "_sync"): "tag-state dict values against a sentinel, not elements",
    ("core/ai.py", "_tidy_groups"): "groups",
    ("core/intersect.py", "intersect"): "polygons of two meshes",
    ("core/links.py", "restamp_materials"): "paint rows: was one copied",
    ("core/outliner.py", "reparent_problem"): "groups",
    ("views/outliner.py", "toggle_isolated"): "groups",
    ("views/tray.py", "_component_steps"): "groups",
    ("views/document_interaction.py", "_cat_kept"): "cached arrays",
    ("views/document_interaction.py", "_epoch_only_matrices"): "draw-record tuples",
    ("views/document_interaction.py", "_epoch_kept_tops"): "draw-record tuples",
}


def _leaf(node) -> str | None:
    while True:
        if isinstance(node, ast.Name):
            return node.id
        if isinstance(node, ast.Attribute):
            return node.attr
        if isinstance(node, ast.Subscript):
            node = node.value
            continue
        if isinstance(node, ast.Call):
            node = node.func
            continue
        return None


def _element_ish(node) -> bool:
    if isinstance(node, ast.Constant):
        return False
    if isinstance(node, ast.Call) and isinstance(node.func, ast.Name) \
            and node.func.id == "type":
        return False
    if isinstance(node, ast.Attribute) and node.attr == "__class__":
        return False
    leaf = _leaf(node)
    return bool(leaf and _ELEMENT.match(leaf)
                and not leaf[:1].isupper())


def _files():
    for d in _DIRS:
        yield from sorted((ROOT / d).rglob("*.py"))
    for name in _FORMAT_FILES:
        p = ROOT / "formats" / name
        if p.exists():
            yield p


def _hits():
    """``(path, enclosing function, line, text)`` of every flagged
    comparison (the outermost function: a nested helper counts as part of
    the function it serves)."""
    def walk(node, rel, fn):
        for child in ast.iter_child_nodes(node):
            inner = fn
            if fn is None and isinstance(
                    child, (ast.FunctionDef, ast.AsyncFunctionDef)):
                inner = child.name
            if isinstance(child, ast.Compare):
                left = child.left
                for op, right in zip(child.ops, child.comparators):
                    if isinstance(op, (ast.Is, ast.IsNot)) \
                            and _element_ish(left) and _element_ish(right):
                        word = "is" if isinstance(op, ast.Is) else "is not"
                        yield (rel, fn, child.lineno, f"{ast.unparse(left)} "
                               f"{word} {ast.unparse(right)}")
                    left = right
            yield from walk(child, rel, inner)

    for path in _files():
        rel = path.relative_to(ROOT).as_posix()
        yield from walk(ast.parse(path.read_text(encoding="utf-8")), rel,
                        None)


def test_no_mesh_element_compared_by_identity():
    bad = [f"{rel}:{line}: {text}" for rel, fn, line, text in _hits()
           if (rel, fn) not in _ALLOWED]
    assert not bad, (
        "mesh elements compared with `is` (a store view is never the same "
        "object twice; use == / !=), or add a reasoned _ALLOWED entry:\n"
        + "\n".join(bad))


def test_the_allowlist_has_no_dead_entries():
    """An entry whose comparison is gone would wave a future one through."""
    seen = {(rel, fn) for rel, fn, _line, _text in _hits()}
    assert not [k for k in _ALLOWED if k not in seen]
