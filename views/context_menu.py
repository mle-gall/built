# SPDX-License-Identifier: GPL-3.0-or-later
"""The viewport right-click menu for the chrome (docs/context-menu.md).

The menu offers only what applies to what was right-clicked, so it has no
More ›. Related verbs share a row as segments (Intersect: Model |
Selection | Context). Each kind keeps to about 14 rows.

This module builds the rows. ``rust/ingetrazo_render/src/ui/context_menu.rs``
draws them and reports ``ctx:<id>``. Nothing here implements a command:

- a verb the project tree also offers runs the tree's own action
  (``wgpu_ui._tree_action`` on the row key ``i:<uid>``), which goes through
  ``core.protocol``;
- every other verb is the window's Edit-menu slot (``_on_reverse_faces``,
  ``_on_intersect_faces(mode)``…), the one the classic QMenu, the menu bar
  and the selection bar call.

Some Qt-host callers still use
``MainWindow.show_viewport_context_menu`` (a QMenu).
"""
from __future__ import annotations

from dataclasses import dataclass, field
from typing import Callable, Optional

#: Kinds, in the order docs/context-menu.md lists them.
KINDS = ("faces", "edges", "geometry", "group", "component", "groups", "mixed",
         "link", "linked_part", "annotation", "guide", "empty",
         "section", "image", "path")
#: Kinds whose model is not this document's to edit.
READ_ONLY = ("link", "linked_part")
#: Verbs a read-only kind must never offer (tests check this list).
EDITING = ("open", "rename", "explode", "makegroup", "makecomp", "fileto", "unique",
           "split", "merge", "intersect", "soften", "flip", "reverse", "orient",
           "heal", "rebuild", "simplify", "replace", "keep", "axes")
#: The most rows a kind may have under its header (Repeat included).
MAX_ROWS = 14


@dataclass
class Verb:
    id: str
    label: str
    run: Optional[Callable] = None
    key: str = ""
    enabled: bool = True
    #: -1: no check; 0 / 1: a check mark, off / on.
    checked: int = -1


@dataclass
class Row:
    verbs: list
    #: A segmented row's family name ("Intersect"); "" for a plain row.
    label: str = ""

    def text(self) -> str:
        """The row as docs/context-menu.md writes it: "Hide",
        "Explode | Make component…", "Intersect: Model | Selection | Context"."""
        verbs = " | ".join(v.label for v in self.verbs)
        return f"{self.label}: {verbs}" if self.label else verbs


@dataclass
class Menu:
    kind: str
    title: str
    subtitle: str
    rows: list = field(default_factory=list)
    #: Where it was opened (logical points, the view's coordinates).
    at: tuple = (0.0, 0.0)

    def verbs(self):
        return [v for r in self.rows for v in r.verbs]

    def find(self, vid):
        return next((v for v in self.verbs() if v.id == vid), None)

    def labels(self) -> list:
        """Each row as text (tests, docs): "Hide", "Intersect: Model | …"."""
        return [r.text() for r in self.rows]

    def ids(self) -> list:
        return [v.id for v in self.verbs()]


# ---- what was hit ------------------------------------------------------------------
def kind_of(scene, sel) -> str:
    """The menu kind for the selection ``sel``."""
    from core.dimension import Dimension
    from core.group import Group
    from core.guide import is_guide
    from core.mesh import Edge, Face
    from core.section import SectionPlane
    from core.textlabel import TextLabel
    from core import links
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
    if all(isinstance(e, (Dimension, TextLabel)) for e in sel):
        return "annotation"
    if all(is_guide(e) for e in sel):
        return "guide"
    if all(isinstance(e, SectionPlane) for e in sel):
        return "section"
    try:
        from core.image_plane import ImagePlane
        if all(isinstance(e, ImagePlane) for e in sel):
            return "image"
    except ImportError:
        pass
    try:
        from georef.geopath import GeoPath
        if all(isinstance(e, GeoPath) for e in sel):
            return "path"
    except ImportError:
        pass
    return "mixed"


def _plural(n, word):
    return f"{n} {word}" + ("" if n == 1 else "s")


def _context_path(scene) -> str:
    """Where loose geometry lives: the groups open for editing, else Model."""
    names = []
    from core.outliner import open_path
    for g in open_path(scene):
        names.append(getattr(g, "name", "") or ("Component" if g is not None and g.is_component() else "Group"))
    return " › ".join(names) if names else "Model"


def _filed_path(scene, g) -> str:
    """The group's node path in the project tree, or "Unfiled"."""
    from core import bim_registry, structure
    try:
        key = structure.node_of(g)
        if key:
            return " › ".join(structure.path(bim_registry.get(scene), key))
    except Exception:  # noqa: BLE001 — a stale key: no path, not an error
        pass
    return "Unfiled"


def _length(value) -> str:
    try:
        from core import units
        return units.fmt_len(float(value))
    except Exception:  # noqa: BLE001
        return f"{float(value):.3f}"


def _area_text(face) -> str:
    try:
        from core import units
        return units.fmt_area(float(face.area()))
    except Exception:  # noqa: BLE001
        return ""


def header(scene, sel, kind) -> tuple:
    """(title, subtitle): what was hit, and where it is filed."""
    from core.group import Group
    from core.mesh import Edge, Face
    from core import links
    sel = list(sel)
    if kind == "empty":
        return "Nothing here", _context_path(scene)
    if kind in ("group", "component"):
        g = sel[0]
        name = getattr(g, "name", "") or ("Component" if kind == "component" else "Group")
        what = "Component" if kind == "component" else "Group"
        if kind == "component":
            n = sum(1 for o in _all_groups(scene) if o.mesh is g.mesh)
            what += f" · {_plural(n, 'instance')}"
        return name, f"{what} · {_filed_path(scene, g)}"
    if kind == "link":
        if len(sel) == 1:
            rec = links.link_of(sel[0]) or {}
            from pathlib import Path
            # "↗" marks a linked file, as in the tree (Inter has no ⛓).
            return f"{Path(rec.get('path', '')).name or sel[0].name} ↗", \
                f"Linked file · read-only · {_filed_path(scene, sel[0])}"
        return _plural(len(sel), "linked file"), "Linked files · read-only"
    if kind == "linked_part":
        g = next(e for e in sel if isinstance(e, Group))
        host = links.linked_ancestor(g)
        from pathlib import Path
        src = Path((links.link_of(host) or {}).get("path", "")).name if host is not None else ""
        name = getattr(g, "name", "") or "Part"
        return name, f"in {src} ↗ · read-only"
    faces = [e for e in sel if isinstance(e, Face)]
    edges = [e for e in sel if isinstance(e, Edge)]
    groups = [e for e in sel if isinstance(e, Group)]
    if kind == "faces":
        if len(faces) == 1:
            return "Face", " · ".join(p for p in (_area_text(faces[0]), "loose geometry", _context_path(scene)) if p)
        return _plural(len(faces), "face"), f"loose geometry · {_context_path(scene)}"
    if kind == "edges":
        if len(edges) == 1:
            e = edges[0]
            try:
                length = _length((e.b - e.a).length())
            except Exception:  # noqa: BLE001
                length = ""
            return "Edge", " · ".join(p for p in (length, "loose geometry", _context_path(scene)) if p)
        return _plural(len(edges), "edge"), f"loose geometry · {_context_path(scene)}"
    if kind == "groups":
        comps = sum(1 for g in groups if g.is_component())
        parts = [p for p in (_plural(len(groups) - comps, "group") if len(groups) > comps else "",
                             _plural(comps, "component") if comps else "") if p]
        return _plural(len(sel), "item"), ", ".join(parts)
    if kind in ("geometry", "mixed"):
        counts = [(len(groups), "group"), (len(faces), "face"), (len(edges), "edge"),
                  (len(sel) - len(groups) - len(faces) - len(edges), "other")]
        return _plural(len(sel), "item"), ", ".join(_plural(n, w) for n, w in counts if n)
    if kind == "annotation":
        from core.dimension import Dimension
        e = sel[0]
        if len(sel) > 1:
            return _plural(len(sel), "annotation"), _context_path(scene)
        if isinstance(e, Dimension):
            try:
                return "Dimension", e.text or e.label()
            except Exception:  # noqa: BLE001
                return "Dimension", ""
        text = (getattr(e, "text", "") or "").strip().splitlines()
        return "Text", f"“{text[0][:40]}”" if text else ""
    if kind == "guide":
        e = sel[0]
        if len(sel) > 1:
            return _plural(len(sel), "guide"), ""
        is_point = getattr(e, "direction", None) is None or type(e).__name__ == "GuidePoint"
        return ("Guide point" if is_point else "Guide line"), "construction"
    if kind == "section":
        return getattr(sel[0], "name", "") or "Section plane", "Section plane"
    if kind == "image":
        return getattr(sel[0], "name", "") or "Image", "Image"
    if kind == "path":
        return "Path", "Geo path"
    return _plural(len(sel), "item"), ""


def _all_groups(scene):
    from core.purge import iter_groups
    return iter_groups(getattr(scene, "groups", None) or ())


# ---- the rows ----------------------------------------------------------------------
def _repeat(win, read_only=False) -> Verb:
    last = getattr(win, "_last_command", None)
    if last is None:
        return Verb("repeat", "Repeat", None, "⇧R", enabled=False)
    label = last[0]
    value = _repeat_value(win, label)
    text = f"Repeat {label}" + (f"  {value}" if value else "")
    return Verb("repeat", text, win.repeat_last_command, "⇧R", enabled=not read_only)


def _repeat_value(win, label) -> str:
    """The value the last command runs with, when it has one (Push/Pull's
    distance, which a double-click repeats)."""
    tools = getattr(win, "_tools", {}) or {}
    for tool in tools.values():
        if getattr(tool, "name", None) != label:
            continue
        value = getattr(type(tool), "last_distance", None)
        if isinstance(value, (int, float)) and value:
            return _length(abs(value))
    return ""


def build(win, view=None, sel=None, *, locked_image=None, locked_group=None) -> Menu:
    """The menu for the current selection (the right-click has already
    selected what it hit, see :func:`open_at`)."""
    vp = win.viewport
    scene = vp.scene
    sel = list(scene.selection if sel is None else sel)
    kind = kind_of(scene, sel)
    title, subtitle = header(scene, sel, kind)
    menu = Menu(kind, title, subtitle)
    rows = menu.rows
    one = lambda *verbs, label="": rows.append(Row([v for v in verbs if v is not None], label))  # noqa: E731
    rows.append(Row([_repeat(win, read_only=kind in READ_ONLY)]))
    # A locked thing under the pointer the click could not select: its
    # Unlock is reachable only from here (#782).
    if locked_group is not None and locked_group not in sel:
        name = getattr(locked_group, "name", "") or ("Component" if locked_group.is_component() else "Group")
        one(Verb("unlockgroup", f"Unlock “{name}”", lambda g=locked_group: win._unlock_group(g)))
    if locked_image is not None and locked_image not in sel:
        name = getattr(locked_image, "name", "") or "image"
        one(Verb("unlockimage", f"Unlock “{name}”", lambda im=locked_image: win._unlock_image(im)),
            Verb("deleteimage", "Delete", lambda im=locked_image: win._delete_image(im)),
            label="Image")
    if kind in ("group", "component") and getattr(sel[0], "text3d", None) and not sel[0].locked:
        one(Verb("edittext3d", "Edit 3D Text…", lambda: win._on_edit_3d_text(sel[0])))
    builder = _BUILDERS.get(kind)
    if builder is not None:
        builder(win, view, scene, sel, one)
    if sel and view is not None:
        from views import wgpu_ui
        from PySide6.QtGui import QKeySequence
        reveal = Verb("revealselection", "Reveal selection", lambda: wgpu_ui._tree_action(win, view, "revealselection" + wgpu_ui.SEP),
                      QKeySequence("Ctrl+Shift+J").toString(QKeySequence.NativeText))
        zoom = next((r for r in rows if any(v.id == "zoom" for v in r.verbs)), None)
        if zoom:
            zoom.verbs.append(reveal)
        else:
            one(reveal)
    menu.rows = [r for r in rows if r.verbs]
    return menu


# ---- per kind ------------------------------------------------------------------------
def _tree(win, view, verb, g, extra=None):
    """Run the project tree's own verb on the item row of ``g``."""
    from views import wgpu_ui
    key = f"{verb}{wgpu_ui.SEP}i:{g.uid}"
    if extra is not None:
        key += f"{wgpu_ui.SEP}{extra}"
    return lambda: wgpu_ui._tree_action(win, view, key)


def _uids(sel):
    return [g.uid for g in sel if getattr(g, "uid", None)]


def _core(win, command, **args):
    from views import wgpu_ui
    return lambda: (wgpu_ui._core(win.viewport, command, **args), win.viewport.update())


def _hide(win, sel):
    return Verb("hide", "Hide", win._on_hide, "") if win._hideable(sel) else None


def _isolate(win, sel):
    from views import wgpu_ui
    uids = _uids(sel)
    return Verb("isolate", "Isolate", lambda: wgpu_ui._isolate(win, uids)) if uids else None


def _zoom(win):
    return Verb("zoom", "Zoom to", win._on_zoom_selection)


def _file_to(win, groups):
    from views import wgpu_ui
    return Verb("fileto", "File to…", lambda: wgpu_ui._file_to(win, len(groups)), "⌘⇧F")


def _make(win, sel):
    return [Verb("makegroup", "Make group", win._on_make_group),
            Verb("makecomp", "Make component…", win._on_make_component)]


def _intersect(win):
    from core.intersect import WITH_CONTEXT, WITH_MODEL, WITH_SELECTION
    return [Verb("intersect:model", "Model", lambda: win._on_intersect_faces(WITH_MODEL)),
            Verb("intersect:selection", "Selection", lambda: win._on_intersect_faces(WITH_SELECTION)),
            Verb("intersect:context", "Context", lambda: win._on_intersect_faces(WITH_CONTEXT))]


def _flip(win):
    return [Verb(f"flip:{a}", label, lambda a=a: win._on_flip_along(a))
            for a, label in (("x", "Red"), ("y", "Green"), ("z", "Blue"))]


def _soften(win):
    return Verb("soften", "Soften / Smooth edges…", win._on_soften_edges)


def _geometry_rows(win, view, scene, sel, one, *, faces, edges):
    """Faces only, edges only, or both (docs/context-menu.md, Raw geometry)."""
    from core.mesh import Edge, Face
    one(_hide(win, sel))
    one(_zoom(win))
    one(*_make(win, sel))
    face_list = [e for e in sel if isinstance(e, Face)]
    edge_list = [e for e in sel if isinstance(e, Edge)]
    if faces:
        one(Verb("reverse", "Reverse faces", win._on_reverse_faces),
            Verb("orient", "Orient faces", win._on_orient_faces))
        one(*_intersect(win), label="Intersect")
        one(_soften(win))
        if edges and win._makes_faces(sel):
            one(Verb("makefaces", "Make faces", win._on_make_faces))
        one(Verb("heal", "Heal overlapping faces", win._on_heal_overlaps))
        one(Verb("rebuild", "Rebuild faces (planar)", win._on_rebuild_planar))
        one(Verb("simplify", "Simplify mesh…", win._on_simplify_mesh))
        one(*_flip(win), label="Flip along")
        if not edges and len(face_list) == 1:
            one(Verb("alignview", "Align view", lambda f=face_list[0]: win._on_align_view_to_face(f)))
            if win._single_textured_face() is not None:
                one(Verb("texpos", "Position", win._on_texture_position),
                    Verb("texreset", "Reset", win._on_texture_reset), label="Texture")
        return
    # Edges only.
    if win._makes_faces(sel):
        one(Verb("makefaces", "Make faces", win._on_make_faces))
    loose = [e for e in edge_list if e in scene.mesh.edges]
    if loose:
        one(Verb("divide", "Divide…", win._on_divide))
        join = Verb("join", "Join segments", win._on_join_segments) if len(loose) > 1 else None
        curve = Verb("explodecurve", "Explode curve", win._on_explode_curve) \
            if any(getattr(e, "curve", None) is not None for e in loose) else None
        one(*(v for v in (join, curve) if v is not None))
    one(_soften(win))
    if win._bridgeable(sel):
        one(Verb("bridge", "Bridge", win._on_bridge))
    one(*_flip(win), label="Flip along")


def _faces(win, view, scene, sel, one):
    _geometry_rows(win, view, scene, sel, one, faces=True, edges=False)


def _edges(win, view, scene, sel, one):
    _geometry_rows(win, view, scene, sel, one, faces=False, edges=True)


def _geometry(win, view, scene, sel, one):
    _geometry_rows(win, view, scene, sel, one, faces=True, edges=True)


def _lock_verb(win, view, groups):
    locked = all(getattr(g, "locked", False) for g in groups)
    label = "Unlock" if locked else "Lock"
    if len(groups) == 1:
        return Verb("lock", label, _tree(win, view, "lockitem", groups[0]))
    return Verb("lock", label, _core(win, "lock", placements=_uids(groups), locked=not locked))


def _rename(win, view, g):
    from views import context_menu as cm
    return Verb("rename", "Rename…", lambda: cm.ask_rename(win, view, g))


def _group(win, view, scene, sel, one):
    g = sel[0]
    one(Verb("open", "Open for editing", _tree(win, view, "open", g), "double-click"))
    one(_rename(win, view, g))
    one(Verb("hide", "Hide", _tree(win, view, "hideitem", g)))
    one(_isolate(win, sel))
    one(_lock_verb(win, view, [g]))
    one(Verb("zoom", "Zoom to", _tree(win, view, "zoom", g)))
    one(_file_to(win, [g]))
    one(Verb("explode", "Explode", win._on_explode_group),
        Verb("makecomp", "Make component…", win._on_make_component))
    if not getattr(g, "billboard", False):
        one(Verb("split", "Split into pieces", win._on_split_into_pieces))
    one(*_intersect(win), label="Intersect")
    one(_soften(win))
    one(*_flip(win), label="Flip along")
    one(Verb("axes", "Change axes", lambda: win._on_change_axes(g)))


def _component(win, view, scene, sel, one):
    from core.kept_components import kept_template
    g = sel[0]
    one(Verb("open", "Open for editing", _tree(win, view, "open", g), "double-click"))
    one(_rename(win, view, g))
    one(Verb("hide", "Hide", _tree(win, view, "hideitem", g)))
    one(_isolate(win, sel))
    one(_lock_verb(win, view, [g]))
    one(Verb("zoom", "Zoom to", _tree(win, view, "zoom", g)))
    one(_file_to(win, [g]))
    one(Verb("instances", "Select instances", _tree(win, view, "instances", g)),
        Verb("unique", "Make unique", _tree(win, view, "unique", g)))
    replace = Verb("replace", "Replace component…", win._on_replace_component) \
        if win._replacement_choices(sel) else None
    one(Verb("explode", "Explode", win._on_explode_group), replace)
    kept = kept_template(scene, g.mesh) is not None
    one(Verb("keep", "Keep in file", lambda: win.keep_selected_components(not kept), checked=int(kept)))
    one(*_intersect(win), label="Intersect")
    one(*_flip(win), label="Flip along")
    one(Verb("axes", "Change axes", lambda: win._on_change_axes(g)))


def _groups(win, view, scene, sel, one):
    from core.solids import is_solid
    groups = list(sel)
    one(_hide(win, sel))
    one(_isolate(win, sel))
    one(_lock_verb(win, view, groups))
    one(_zoom(win))
    one(_file_to(win, groups))
    one(*_make(win, sel))
    one(Verb("merge", "Merge groups", win._on_merge_groups),
        Verb("explode", "Explode", win._on_explode_group))
    if any(g.is_component() for g in groups):
        one(Verb("unique", "Make unique", win._on_make_unique))
    one(*_intersect(win), label="Intersect")
    solid = [g for g in groups if is_solid(g)]
    if len(solid) >= 2 and len(solid) == len(groups):
        keys = [("solid_union", "Union"), ("solid_intersect", "Intersect")]
        if len(solid) == 2:
            keys.append(("solid_split", "Split"))
        keys.append(("outer_shell", "Outer shell"))
        one(*(Verb(f"solid:{k}", label, lambda k=k: win._activate_tool(k)) for k, label in keys),
            label="Solid")
    one(_soften(win))
    one(*_flip(win), label="Flip along")


def _mixed(win, view, scene, sel, one):
    from core.group import Group
    from core.mesh import Face
    groups = [e for e in sel if isinstance(e, Group)]
    one(_hide(win, sel))
    one(_isolate(win, groups))
    one(_zoom(win))
    if groups:
        one(_file_to(win, groups))
    one(*_make(win, sel))
    if any(isinstance(e, Face) for e in sel):
        one(Verb("reverse", "Reverse faces", win._on_reverse_faces),
            Verb("orient", "Orient faces", win._on_orient_faces))
    one(*_intersect(win), label="Intersect")
    one(_soften(win))
    one(*_flip(win), label="Flip along")


def _link(win, view, scene, sel, one):
    from core import links
    from views import wgpu_ui
    g = sel[0]
    contents = lambda: wgpu_ui._select(win, [c for p in sel for c in getattr(p, "children", None) or ()  # noqa: E731
                                             if scene.entity_selectable(c)] or list(sel))
    one(Verb("contents", "Select contents", contents))
    one(Verb("hide", "Hide", _core(win, "hide", placements=_uids(sel))))
    one(_isolate(win, sel))
    # The placement's padlock is the host's, not the linked file's.
    one(_lock_verb(win, view, list(sel)))
    one(_zoom(win))
    one(Verb("linkopen", "Open source", lambda: wgpu_ui._link_action(win, view, g, "linkopen")))
    one(Verb("linkreload", "Reload", lambda: wgpu_ui._link_action(win, view, g, "linkreload")))
    if len(sel) == 1:
        nested = bool((links.link_of(g) or {}).get("include_nested", True))
        one(Verb("linknested", "Include nested files", lambda: wgpu_ui._link_action(win, view, g, "linknested"),
                 checked=int(nested)))


def _linked_part(win, view, scene, sel, one):
    from core import links
    from core.group import Group
    from views import wgpu_ui
    part = next(e for e in sel if isinstance(e, Group))
    host = links.linked_ancestor(part)
    if host is not None:
        one(Verb("selectlink", "Select its linked file", lambda: wgpu_ui._select(win, [host])))
    one(_isolate(win, sel))
    one(_zoom(win))
    if host is not None:
        one(Verb("linkopen", "Open source", lambda: wgpu_ui._link_action(win, view, host, "linkopen")))
        one(Verb("linkreload", "Reload", lambda: wgpu_ui._link_action(win, view, host, "linkreload")))


def _annotation(win, view, scene, sel, one):
    if len(sel) == 1:
        one(Verb("edittext", "Edit text…", lambda: edit_text(win, view, sel[0])))
    one(_zoom(win))


def _guide(win, view, scene, sel, one):
    from core.history import DeleteGuidesCommand

    def delete_these():
        win.viewport.history.execute(DeleteGuidesCommand(list(sel)))
        win.viewport.update()
    one(Verb("deleteguide", "Delete guide" if len(sel) == 1 else "Delete guides", delete_these))
    one(Verb("deleteguides", "Delete all guides", win._on_delete_guides))
    one(_zoom(win))


def _empty(win, view, scene, sel, one):
    from formats import clip as clip_transfer
    from views import wgpu_ui
    vp = win.viewport
    if getattr(vp, "clipboard", None) or clip_transfer.available():
        one(Verb("paste", "Paste here", win._on_paste, "⌘V"))
    one(Verb("selectall", "Select all", win._on_select_all, "⌘A"))
    one(Verb("unhideall", "Unhide all", win._on_unhide_all))
    if vp.__dict__.get("_isolate_base") is not None or getattr(vp, "_previous_views", None):
        one(Verb("showall", "Show all", lambda: wgpu_ui._show_all(vp)))
    one(Verb("zoomextents", "Zoom extents", win._on_zoom_extents, "⇧⌘E"))
    if list(getattr(scene, "guides", ()) or ()):
        one(Verb("deleteguides", "Delete all guides", win._on_delete_guides))


def _section(win, view, scene, sel, one):
    p = sel[0]
    one(Verb("secreverse", "Reverse", win._on_reverse_section),
        Verb("secactive", "Active cut", win._on_toggle_active_section, checked=int(bool(p.active))),
        Verb("secalign", "Align view", win._on_align_view_to_section), label="Section")
    one(Verb("secname", "Name and symbol…", lambda: win.prompt_section_name(p)))
    one(_zoom(win))


def _image(win, view, scene, sel, one):
    one(Verb("imgsize", "Size…", win._on_image_size),
        Verb("imgopacity", "Opacity…", win._on_image_opacity), label="Image")
    one(Verb("imgexplode", "Explode image", win._on_explode_image))
    one(Verb("imglock", "Lock", win._on_toggle_image_lock, checked=int(bool(sel[0].locked))))
    one(_zoom(win))


def _path(win, view, scene, sel, one):
    one(Verb("profile", "Terrain profile", win._on_terrain_profile))
    if any(len(getattr(e, "points", ()) or ()) >= 3 for e in sel):
        one(Verb("surf:flat", "Flat", lambda: win._on_set_surface("flat")),
            Verb("surf:draped", "Draped", lambda: win._on_set_surface("draped")),
            Verb("surf:none", "None", lambda: win._on_set_surface(None)), label="Terrain surface")
    one(Verb("convertpath", "Convert path to geometry", win._on_convert_geopath))
    one(Verb("closepath", "Open / close path", win._on_toggle_path_closed))


_BUILDERS = {"faces": _faces, "edges": _edges, "geometry": _geometry, "group": _group,
             "component": _component, "groups": _groups, "mixed": _mixed, "link": _link,
             "linked_part": _linked_part, "annotation": _annotation, "guide": _guide,
             "empty": _empty, "section": _section, "image": _image, "path": _path}


# ---- verbs that need a field or a dialog ------------------------------------------
def ask_rename(win, view, g) -> None:
    """Rename… : the chrome's name prompt (views/chrome_forms.py), answered
    into the tree's own rename (``rename`` on the row ``i:<uid>``, core
    ``rename_placement``)."""
    from views import chrome_forms
    chrome_forms.ask_text(win, "Rename", "Name:", text=getattr(g, "name", "") or "",
                          done=lambda text: rename_answered(win, view, g, text))


def rename_answered(win, view, g, text) -> None:
    """The prompt's Rename: the tree's verb on the item's row."""
    text = (text or "").strip()
    if text and g is not None:
        _tree(win, view, "rename", g, text)()


def edit_text(win, view, entity) -> None:
    """Edit text… : what a double-click on the text or dimension does (the
    Select tool's own handler), at the point the menu was opened on."""
    from PySide6.QtCore import QPointF, Qt
    from ingetrazo_core import Vec3 as QVector3D
    from tools.base import ToolContext
    from tools.select import SelectTool
    vp = win.viewport
    px = getattr(vp, "_context_pixel", None)
    if px is None:
        return
    tools = getattr(view, "tools", None) or vp
    SelectTool().on_double_click(ToolContext(
        tools, QVector3D(), QPointF(float(px[0]), float(px[1])), Qt.NoModifier, None))


# ---- opening it ------------------------------------------------------------------------
def open_at(view, x, y) -> Optional[Menu]:
    """The right-click at ``(x, y)``: select what is under the pointer if it
    is not selected yet (``Viewport._context_pick``), build the menu for the
    selection, and hand it to the chrome. Returns the menu."""
    source = view.source_viewport
    if source is None:
        return None
    win = source.window()
    source._context_pixel = (x, y)
    tools = view.tools
    picked = tools._context_pick(x, y)
    # A locked image or group under the pointer, which no click selects:
    # its Unlock lives here (as in the classic menu).
    under = tools.pick_locked_image_plane(x, y)
    locked_image = under if under is not picked else None
    locked_group = tools.pick_locked_group(x, y)
    from ingetrazo_render import context_targets
    sel = context_targets(picked is not None, source.scene.selection)
    menu = build(win, view, sel=sel, locked_image=locked_image, locked_group=locked_group)
    menu.at = (float(x), float(y))
    view._ctx_menu = menu
    send(view, menu)
    return menu


def send(view, menu) -> None:
    renderer = getattr(view, "renderer", None)
    if renderer is None or not hasattr(renderer, "ui_set_context_menu"):
        return
    rows = [(r.label, [(v.id, v.label, v.key, bool(v.enabled and v.run is not None), int(v.checked))
                       for v in r.verbs]) for r in menu.rows]
    renderer.ui_set_context_menu(float(menu.at[0]), float(menu.at[1]), menu.title, menu.subtitle, rows)
    view.update()


def action(win, view, vid) -> bool:
    """``ctx:<id>`` from the chrome: run that verb of the open menu."""
    menu = getattr(view, "_ctx_menu", None) if view is not None else None
    view._ctx_menu = None
    verb = menu.find(vid) if menu is not None else None
    if verb is None or verb.run is None or not verb.enabled:
        return False
    verb.run()
    try:
        view.tools.sync()
    except Exception:  # noqa: BLE001
        pass
    win.viewport.update()
    return True


def chrome_menu_available(view) -> bool:
    """Whether this view's chrome draws the right-click menu (the native
    shell, or the Qt app on Metal)."""
    renderer = getattr(view, "renderer", None)
    return (getattr(view, "_ui_attached", False)
            and hasattr(renderer, "ui_set_context_menu") and hasattr(renderer, "ui_over_chrome"))
