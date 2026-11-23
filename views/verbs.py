# SPDX-License-Identifier: GPL-3.0-or-later
"""Which verbs apply to what is selected: one table for the inspector's
chips, the selection bar and the viewport's right-click menu (tree C,
docs/ui-decisions.md, 2026-11-23: "the inspector offers only verbs that
apply").

The kinds and the read-only rule are the ones ``docs/context-menu.md``
(branch ``shell/tabs-menu``) lays out for the right-click menu, so that
both surfaces agree: a linked file's placement or a part inside one is
read-only here (no Explode, no Edit, no File to, no editable name), raw
geometry cannot be filed and does not become a component from the
inspector, a component instance is already one. The menu has more rows per
kind (Repeat, Rename…, Flip…); the inspector keeps the few that a
selection needs and puts geometry fixes under More.

Verb ids are the menu's (``open`` is "Open for editing", the inspector's
Edit chip). Nothing here runs a command: ``views/wgpu_ui._commands`` and
the menu map ids to their actions.
"""
from __future__ import annotations

#: Kinds, in docs/context-menu.md's order.
KINDS = ("faces", "edges", "geometry", "group", "component", "groups", "mixed",
         "link", "linked_part", "annotation", "guide", "empty",
         "section", "image", "path")
#: Kinds whose model is not this document's to edit.
READ_ONLY = ("link", "linked_part")
#: Verbs that edit the selected model: never offered on a read-only kind.
EDITING = ("open", "rename", "explode", "makegroup", "makecomp", "fileto", "unique",
           "split", "merge", "intersect", "soften", "flip", "reverse", "orient",
           "heal", "rebuild", "simplify", "replace", "keep", "axes", "join", "topath",
           "status", "repair")

_FACE_FIXES = ("intersect", "soften", "reverse", "orient", "heal", "rebuild", "simplify")
_EDGE_FIXES = ("intersect", "soften", "join", "topath")
_GROUP_FIXES = ("intersect", "soften", "reverse", "orient", "heal", "rebuild", "split", "simplify")

#: The inspector's verbs per kind, in chip order; the fixes go under More.
INSPECTOR = {
    "faces": ("isolate", "hide", "makegroup") + _FACE_FIXES,
    "edges": ("isolate", "hide", "makegroup") + _EDGE_FIXES,
    "geometry": ("isolate", "hide", "makegroup") + tuple(dict.fromkeys(_FACE_FIXES + _EDGE_FIXES)),
    "group": ("fileto", "isolate", "hide", "open", "explode", "makecomp", "status", "repair") + _GROUP_FIXES,
    "component": ("fileto", "isolate", "hide", "open", "explode", "unique", "status", "repair") + _GROUP_FIXES,
    "groups": ("fileto", "isolate", "hide", "makegroup", "makecomp", "status") + _GROUP_FIXES,
    "mixed": ("fileto", "isolate", "hide", "makegroup") + _FACE_FIXES,
    "link": ("isolate", "hide", "zoom", "linkopen", "linkreload", "linkunlink", "linkfinder",
             "linkrelink", "linkpack", "linknested", "contents"),
    "linked_part": ("isolate", "hide", "zoom", "linkfile", "linkopen"),
    "annotation": ("hide",),
    "guide": ("guides",),
    "empty": (),
    "section": ("isolate", "hide"),
    "image": ("isolate", "hide"),
    "path": ("isolate", "hide", "topath"),
}
#: Verbs that live under More ▾ rather than as a chip.
MORE = frozenset(_FACE_FIXES + _GROUP_FIXES + _EDGE_FIXES + ("guides",))

#: A linked file's row in the tree: the file verbs, by whether the host
#: places it itself (top) or it comes inside another file (nested, "via").
FILE_VERBS = {
    "top": ("linkopen", "linkreload", "linkunlink", "linkfinder"),
    "nested": ("linkopen", "linkfinder"),
}


def kind_of(scene, sel) -> str:
    """The kind of the selection ``sel`` (docs/context-menu.md)."""
    from core import links
    from core.group import Group
    from core.mesh import Edge, Face
    sel = list(sel)
    if not sel:
        return "empty"
    groups = [e for e in sel if isinstance(e, Group)]
    if groups:
        if any(links.linked_ancestor(g) is not None and not links.is_linked(g) for g in groups):
            return "linked_part"
        if all(links.is_linked(g) for g in groups) and len(groups) == len(sel):
            return "link"
    faces = sum(1 for e in sel if isinstance(e, Face))
    edges = sum(1 for e in sel if isinstance(e, Edge))
    if groups and len(groups) == len(sel):
        if len(groups) == 1:
            return "component" if groups[0].is_component() else "group"
        return "groups"
    if faces + edges == len(sel):
        return "faces" if not edges else "edges" if not faces else "geometry"
    if groups and len(groups) + faces + edges == len(sel):
        return "mixed"
    for test, kind in ((_annotation, "annotation"), (_guide, "guide"), (_section, "section"),
                       (_image, "image"), (_path, "path")):
        if all(test(e) for e in sel):
            return kind
    return "mixed"


def _annotation(e) -> bool:
    return type(e).__name__ in ("Dimension", "TextLabel")


def _guide(e) -> bool:
    try:
        from core.guide import is_guide
        return bool(is_guide(e))
    except Exception:  # noqa: BLE001
        return False


def _section(e) -> bool:
    return type(e).__name__ == "SectionPlane"


def _image(e) -> bool:
    return type(e).__name__ == "ImagePlane"


def _path(e) -> bool:
    return type(e).__name__ == "GeoPath"


def read_only(kind) -> bool:
    return kind in READ_ONLY


def offers(kind, verb) -> bool:
    """Whether the inspector offers ``verb`` on a selection of ``kind``."""
    if kind in READ_ONLY and verb in EDITING:
        return False
    return verb in INSPECTOR.get(kind, ())


def inspector_verbs(kind) -> tuple:
    return tuple(v for v in INSPECTOR.get(kind, ()) if offers(kind, v))


def name_editable(kind) -> bool:
    """A name field edits in place only on this document's own things."""
    return kind not in READ_ONLY and kind != "empty"


def file_verbs(nested: bool) -> tuple:
    return FILE_VERBS["nested" if nested else "top"]
