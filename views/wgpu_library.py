# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The Library: one place in the dock for every kind of reusable resource
(Maxence 2026-11-22: "everything is now into library but we have no
separation, sorting, ordering by types of resources").

Each resource type is a *provider* in ``PROVIDERS``: plain data in, plain
data out, so the registry can move to Rust with the rest of the app
(docs/rust-port-plan.md). A provider lists items as dicts::

    {"key": "m:Beech", "name": "Beech", "source": "model" | "linked" | "mine" | "bundled",
     "category": "Wood", "uses": 12, "hex": "#a0703c", "thumb": "/path.png" | None,
     "verbs": ("apply", "edit", ...)}

and runs a verb on one (``run``). This module keeps the browsing state
(type, sort, facet, filter, folded sections, recent picks) and answers the
``lib:<verb>\\x1f<key>[\\x1f<arg>]`` actions; what the sheet lists, how it
sorts, filters and groups, each tile's words and a material's details come
from ``rust/ingetrazo_render/src/library_view.rs``. Model edits go
through ``core.protocol.commands`` (docs/core-protocol.md §1).

The dock's places: Project (the tree, with Problems as one of its modes)
and Library, switched in the dock header. Opening the Library remembers
where Project was; the switch or Esc goes back there."""
from __future__ import annotations

import json
from functools import lru_cache
import re
from collections import Counter

from core.settings import settings as QSettings

SEP = "\x1f"
from ingetrazo_render import (library_by_use, library_filtered, library_groups, library_linked,  # noqa: E402
                              library_sheet, library_side, library_type, library_verbs)

SORTS = library_type("")[0]
#: Verb ids and their menu labels (a type relabels "apply").
VERBS = dict(library_verbs("")) | {"-": ""}
RECENT_MAX = 40
ROW_CAP = 600
THUMB_PX = 96
_MINE_KEY = "library/mine"


def _hex(rgb) -> str:
    return "#%02x%02x%02x" % tuple(int(max(0.0, min(1.0, float(c))) * 255) for c in rgb[:3]) if rgb else ""


def _parse_hex(text):
    t = (text or "").strip().lstrip("#")
    if len(t) != 6:
        raise ValueError("A colour is six hex digits, like #a0703c")
    v = int(t, 16)
    return ((v >> 16 & 255) / 255.0, (v >> 8 & 255) / 255.0, (v & 255) / 255.0)


def _settings_json(key, default):
    try:
        raw = QSettings().value(key)
        return json.loads(raw) if isinstance(raw, str) and raw else default
    except (ValueError, TypeError):
        return default


def _store_json(key, value) -> None:
    try:
        QSettings().setValue(key, json.dumps(value))
    except (TypeError, ValueError):
        pass


class Ctx:
    """What a provider reads: the window, the view and the open scene."""

    def __init__(self, win, view):
        self.win, self.view = win, view
        self.vp = win.viewport
        self.scene = win.viewport.scene

    def core(self, command, **args):
        from views.wgpu_ui import _core
        return _core(self.vp, command, **args)

    def flash(self, text) -> None:
        self.vp.flash_status(text)


# ---- usage --------------------------------------------------------------------

def material_uses(scene) -> Counter:
    """Objects wearing each registry material: one per mesh with a face (front
    or back) in it, plus one per container painted whole. Kept per content
    version; a store-backed mesh answers from its rows."""
    sig = (id(scene), scene.content_version)
    cached = getattr(scene, "_lib_uses", None)
    if cached is not None and cached[0] == sig:
        return cached[1]
    from core.purge import iter_groups
    from core.purge import _document_groups, iter_meshes
    from core.store_parts import material_names
    uses = Counter()
    for mesh in iter_meshes(scene):
        names = material_names(mesh)
        if names is None:
            names = set()
            for f in mesh.faces:
                names.add(f.attrs.get("mat"))
                back = f.attrs.get("back")
                if isinstance(back, dict):
                    names.add(back.get("mat"))
        for n in names:
            if n:
                uses[n] += 1
    for top in _document_groups(scene):
        for g in iter_groups((top,)):
            paint = getattr(g, "material", None) or {}
            if paint.get("mat"):
                uses[paint["mat"]] += 1
    scene._lib_uses = (sig, uses)
    return uses


def linked_material_uses(scene) -> dict:
    """{linked file: Counter(material → objects wearing it)}, per content
    version: what each linked file's parts are painted with."""
    sig = (id(scene), scene.content_version)
    cached = getattr(scene, "_lib_linked", None)
    if cached is not None and cached[0] == sig:
        return cached[1]
    out = {}
    try:
        from core import links
        from core.purge import iter_groups
        from core.store_parts import material_names
        for path in links.sources(scene):
            counts = Counter()
            for g in links.linked_groups(scene, path):
                for pg in iter_groups((g,)):
                    mesh = getattr(pg, "mesh", None)
                    if mesh is None:
                        continue
                    names = material_names(mesh)
                    if names is None:
                        names = {f.attrs.get("mat") for f in mesh.faces}
                    for n in names:
                        if n:
                            counts[n] += 1
            if counts:
                out[path] = counts
    except Exception:  # noqa: BLE001 — no link data: nothing read-only to list
        out = {}
    scene._lib_linked = (sig, out)
    return out


def _wears(mesh, name, memo) -> bool:
    if id(mesh) not in memo:
        from core.store_parts import material_names
        names = material_names(mesh)
        if names is None:
            names = {f.attrs.get("mat") for f in mesh.faces}
        memo[id(mesh)] = names
    return name in memo[id(mesh)]


def select_material_uses(ctx, name) -> int:
    """Select what wears ``name`` in the open context: its loose faces and
    each placement with a face or a part painted with it."""
    from core.purge import iter_groups
    scene, memo = ctx.scene, {}
    picked = [f for f in scene.mesh.faces
              if f.attrs.get("mat") == name or (f.attrs.get("back") or {}).get("mat") == name]
    for g in scene.groups:
        for pg in iter_groups((g,)):
            if (getattr(pg, "material", None) or {}).get("mat") == name or _wears(pg.mesh, name, memo):
                picked.append(g)
                break
    picked = [e for e in picked if not hasattr(e, "uid") or scene.entity_selectable(e)]
    scene.select(picked)
    getattr(ctx.vp, "notify_scene_changed", ctx.vp.update)()
    return len(picked)


# ---- providers ----------------------------------------------------------------

class Provider:
    """One resource type. ``ready`` False shows the tab as coming."""
    id = ""
    label = ""
    ready = True

    @property
    def empty(self) -> str:
        return library_type(self.id)[3]

    @property
    def facets(self) -> list:
        return library_type(self.id)[1]

    def items(self, ctx) -> list[dict]:
        return []

    def verbs(self) -> list[tuple[str, str]]:
        return library_verbs(self.id)

    def type_actions(self, ctx) -> list[tuple[str, str]]:
        return []

    def run(self, ctx, verb, item, arg="") -> None:
        pass

    def type_action(self, ctx, action) -> None:
        pass

    def inspector(self, ctx, item):
        return None

    def edit(self, ctx, item, field, text) -> None:
        pass


def _file_stem(path) -> str:
    return re.sub(r"^[0-9a-f]{12,}-", "", str(path).replace("\\", "/").rsplit("/", 1)[-1])


#: PBR rows of the editor: (channel, label, map channel).
PBR_ROWS = [("metallic", "Metallic", "metallic_map"), ("roughness", "Roughness", "roughness_map"),
            ("normal_scale", "Normal", "normal_map"), ("ao_strength", "Occlusion", "ao_map")]


class MaterialsProvider(Provider):
    """Every paint is a Material (Maxence 2026-11-22: "do colors really need
    to be separate from materials?"): a colour or an image as its base, the
    same PBR values and opacity on top. The model's registry, its unnamed
    colours (older files), My library, then the bundled images by category
    and the bundled colours (RAL) as one more category."""
    id, label = "materials", "Materials"

    def _model_items(self, ctx):
        uses = material_uses(ctx.scene)
        out = []
        for name, mat in (getattr(ctx.scene, "materials", None) or {}).items():
            tex = (mat.texture or {}).get("path")
            out.append({"key": f"m:{name}", "name": name, "source": "model", "category": "",
                        "base": "image" if tex else "colour",
                        "uses": uses.get(name, 0), "hex": _hex(mat.color), "thumb": tex,
                        "verbs": ("apply", "edit", "select", "-", "duplicate", "rename", "keep", "-", "delete")})
        for key, entry in unnamed_colours_cached(ctx.scene).items():
            n = len(entry["faces"]) + len(entry["groups"])
            out.append({"key": f"uc:{key}", "name": key.split("@")[0], "source": "model",
                        "category": "Unnamed colours", "base": "colour", "uses": n,
                        "hex": _hex(entry["color"]), "thumb": None, "verbs": ("apply", "select", "-", "name")})
        return out

    def _mine_items(self, ctx):
        out = []
        for i, e in enumerate(_settings_json(_MINE_KEY, [])):
            if not isinstance(e, dict) or e.get("type") not in (self.id, "colours"):
                continue
            tex = (e.get("texture") or {}).get("path")
            out.append({"key": f"my:{i}", "name": e.get("name", ""), "source": "mine",
                        "category": e.get("category", ""), "base": "image" if tex else "colour",
                        "uses": material_uses(ctx.scene).get(e.get("name"), 0),
                        "hex": _hex(e.get("color")), "thumb": tex, "verbs": ("apply", "-", "unkeep")})
        return out

    def _bundled(self, ctx):
        return list(_catalogue().values())

    def _linked_items(self, ctx, model):
        """The linked files' materials, read-only, under each file: the
        registry materials that only linked parts wear move out of «In this
        model» (editing them here would change a file that is not this one)."""
        per_file = linked_material_uses(ctx.scene)
        if not per_file:
            return model, []
        keep, linked = library_linked(json.dumps(model), json.dumps([(p, list(c.items())) for p, c in per_file.items()]),
                                      json.dumps(dict(material_uses(ctx.scene))))
        return json.loads(keep), json.loads(linked)

    def items(self, ctx):
        if getattr(self, "_bundled_cache", None) is None:
            self._bundled_cache = self._bundled(ctx)
        model, linked = self._linked_items(ctx, self._model_items(ctx))
        return model + linked + self._mine_items(ctx) + [dict(e) for e in self._bundled_cache]

    def type_actions(self, ctx):
        from core.purge import unused_materials
        n = len(unused_materials(ctx.scene))
        acts = [("purge", f"Purge unused materials ({n})" if n else "Purge unused materials (none)")]
        k = len(unnamed_colours_cached(ctx.scene))
        if k:
            acts.insert(0, ("name", f"Name these: {k} unnamed colour" + ("s" if k != 1 else "")))
        return acts

    def type_action(self, ctx, action):
        if action == "purge":
            out = ctx.core("purge_unused", layers=False, materials=True)
            ctx.flash("Unused materials purged" if out and out.get("ok") else "Every material is in use")
        elif action == "name":
            out = ctx.core("name_unnamed_colours") or {}
            n = out.get("named", 0)
            ctx.flash(f"{n} colour{'s' if n != 1 else ''} named: each is a material now" if n
                      else "Every colour already has a name")

    def _mine_entry(self, item):
        i = int(item["key"].partition(":")[2])
        return _settings_json(_MINE_KEY, [])[i]

    def paint_recipe(self, ctx, item):
        key = item["key"]
        kind = key.partition(":")[0]
        if kind == "uc":
            return "", unnamed_colours_cached(ctx.scene).get(key[3:])
        if kind == "my":
            return "", self._mine_entry(item)
        if kind in ("m", "lk"):
            return item["name"], None
        return "", item.get("recipe") or _catalogue().get(key, {}).get("recipe")

    def run(self, ctx, verb, item, arg=""):
        key, name = item["key"], item["name"]
        kind = key.partition(":")[0]
        if verb == "apply":
            if kind in ("uc", "my"):
                _, recipe = self.paint_recipe(ctx, item)
                if recipe:
                    _pick_paint(ctx.win, recipe=recipe)
            else:
                from views.wgpu_ui import _paint_with
                _paint_with(ctx.win, f"m:{name}" if kind == "lk" else key)
        elif verb == "name":
            self.type_action(ctx, "name")
        elif kind == "uc":
            entry = unnamed_colours_cached(ctx.scene).get(key[3:])
            if entry is None:
                return
            if verb == "select":
                loose = set(map(id, ctx.scene.mesh.faces))
                faces = [f for f, side in entry["faces"] if side == "front" and id(f) in loose]
                ctx.scene.select(faces + [g for g in entry["groups"] if g in ctx.scene.groups])
                getattr(ctx.vp, "notify_scene_changed", ctx.vp.update)()
        elif verb == "edit" and kind == "m":
            set_mode(ctx.view, ctx.win, "")
            ctx.view._ui_mat_focus = key
            if hasattr(ctx.view.renderer, "ui_show_inspector"):
                ctx.view.renderer.ui_show_inspector()
        elif verb == "select" and kind in ("m", "lk"):
            n = select_material_uses(ctx, name)
            ctx.flash(f"{n} selected wearing {name}" if n else f"Nothing in the open context wears {name}")
        elif verb == "duplicate" and kind == "m":
            out = ctx.core("duplicate_material", name=name)
            if out and out.get("name"):
                ctx.view._ui_mat_focus = f"m:{out['name']}"
                _state(ctx.view)["cursor"] = f"m:{out['name']}"
        elif verb == "rename" and kind == "m" and arg.strip() and arg.strip() != name:
            ctx.core("rename_material", name=name, new_name=arg.strip())
            _rename_paint(ctx.scene, name, arg.strip())
            if ctx.view.__dict__.get("_ui_mat_focus") == key:
                ctx.view._ui_mat_focus = f"m:{arg.strip()}"
        elif verb == "delete" and kind == "m":
            n = item.get("uses", 0)
            body = (f"{n} object{'s' if n != 1 else ''} wearing it go back to the default material."
                    if n else "Nothing wears it.")
            _ask(ctx.view, ("delete", self.id, key), f"Delete “{name}”?", body, "Delete")
        elif verb == "keep" and kind == "m":
            mat = ctx.scene.materials.get(name)
            if mat is not None:
                mine = _settings_json(_MINE_KEY, [])
                mine = [e for e in mine if not (e.get("type") in (self.id, "colours") and e.get("name") == name)]
                mine.append({"type": self.id, **mat.to_dict()})
                _store_json(_MINE_KEY, mine)
                _state(ctx.view)["mine_rev"] += 1
                ctx.flash(f"{name} is in My library, for every project")
        elif verb == "unkeep" and kind == "my":
            mine = _settings_json(_MINE_KEY, [])
            i = int(key.partition(":")[2])
            if 0 <= i < len(mine):
                mine.pop(i)
                _store_json(_MINE_KEY, mine)
                _state(ctx.view)["mine_rev"] += 1
        elif verb in ("map", "unmap") and kind == "m":
            channel = arg if arg in {m for _c, _l, m in PBR_ROWS} else ""
            if not channel:
                return
            if verb == "unmap":
                ctx.core("set_material_pbr", name=name, **{channel: None})
                return
            label = next(lab for _c, lab, m in PBR_ROWS if m == channel)
            path = _pick_image(ctx.win, f"{label} map")
            if path:
                values = {channel: {"path": path}}
                scalar = next(c for c, _l, m in PBR_ROWS if m == channel)
                mat = ctx.scene.materials.get(name)
                if mat is not None and scalar not in (mat.pbr or {}):
                    values[scalar] = 1.0      # a map with no strength would show nothing
                ctx.core("set_material_pbr", name=name, **values)
        elif verb == "image" and kind == "m":
            path = _pick_image(ctx.win, "Base image")
            if path:
                mat = ctx.scene.materials.get(name)
                tex = dict((mat.texture or {}) if mat else {})
                tex.update(path=path)
                tex.pop("embed", None)
                ctx.core("set_material_base", name=name, base="image", texture=tex)

    def confirmed(self, ctx, what, key, text=""):
        if what == "delete":
            ctx.core("delete_material", name=key.partition(":")[2])
            ctx.view._ui_mat_focus = None

    def inspector(self, ctx, item):
        key = item["key"]
        if key.startswith("uc:"):
            n = item.get("uses", 0)
            rows = [("Colour", item["name"]), ("Uses", str(n)), ("Name", "none (an older file)")]
            k = len(unnamed_colours_cached(ctx.scene))
            acts = [(f"lib:name:{key}", f"Name these ({k})"), (f"lib:select:{key}", "Select uses"),
                    (f"lib:apply:{key}", "Paint with it")]
            return "Unnamed colour", [("Colour without a material", rows), ("Actions", acts)]
        if not key.startswith("m:"):
            return None
        mat = ctx.scene.materials.get(item["name"])
        if mat is None:
            return None
        from ingetrazo_core import material_inspector
        from views.wgpu_ui import _active_tool_id, _current_material_name
        return material_inspector(mat, item.get("uses", 0),
                                  _active_tool_id(ctx.win) == "paint" and _current_material_name() == mat.name,
                                  ctx.view.__dict__.get("_ui_mat_focus") == key)

    def edit(self, ctx, item, field, text):
        name = item["name"]
        text = (text or "").strip()
        if field == "name":
            self.run(ctx, "rename", item, text)
        elif field == "base":
            if text.lower().startswith("image"):
                mat = ctx.scene.materials.get(name)
                if mat is not None and (mat.texture or {}).get("path"):
                    return
                self.run(ctx, "image", {**item, "key": f"m:{name}"})
            else:
                ctx.core("set_material_base", name=name, base="color")
        elif field in ("color", "finish") or field.startswith("brush_"):
            mat = ctx.scene.materials.get(name)
            if field == "color" and mat is not None and (mat.texture or {}).get("path"):
                ctx.core("edit_material", name=name, color=list(_parse_hex(text)))
                return
            from ingetrazo_core import edit_swatch
            from tools.paint import PaintTool
            new = edit_swatch(ctx.vp.history, PaintTool, name, field, text)
            if ctx.view is not None and ctx.view.__dict__.get("_ui_mat_focus") == f"m:{name}":
                ctx.view._ui_mat_focus = f"m:{new}"
            ctx.vp.refresh_after_edit()
        elif field == "tile":
            parts = [p for p in re.split(r"[^0-9.,]+", text.replace(",", ".")) if p]
            if not parts:
                raise ValueError("Type the tile size, like 1.2 × 0.6")
            w = float(parts[0])
            h = float(parts[1]) if len(parts) > 1 else w
            if w <= 0 or h <= 0:
                raise ValueError("A tile is larger than zero")
            ctx.core("edit_material", name=name, tile=[w, h])
        elif field == "pbr" or field.startswith("percent_"):
            from ingetrazo_core import material_surface_input
            ctx.core("set_material_pbr", name=name, **material_surface_input(field, text))
        elif field == "opacity":
            v = float(text.rstrip("%"))
            if not 0 <= v <= 100:
                raise ValueError("Opacity is between 0 and 100 %")
            ctx.core("edit_material", name=name, opacity=v / 100.0)
        elif field in {c for c, _l, _m in PBR_ROWS}:
            ctx.core("set_material_pbr", name=name, **{field: float(text.replace(",", ".")) if text else None})


def unnamed_colours_cached(scene) -> dict:
    sig = (id(scene), scene.content_version)
    cached = getattr(scene, "_lib_unnamed", None)
    if cached is None or cached[0] != sig:
        from core.material_commands import unnamed_colours
        cached = (sig, unnamed_colours(scene))
        scene._lib_unnamed = cached
    return cached[1]


def _pick_image(win, title):
    from views.filedialogs import file_dialogs
    path, _ = file_dialogs.getOpenFileName(win, title, "", "Images (*.png *.jpg *.jpeg *.webp *.bmp *.tif *.tiff)")
    return path or None


class ComponentsProvider(Provider):
    """Definitions in the model (copies counted, kept-in-file ones too),
    then the bundled starters and 2D people."""
    id, label = "components", "Components"

    def _model(self, ctx):
        from views.wgpu_ui import _components_in_model
        return _components_in_model(ctx.view, ctx.win)

    def items(self, ctx):
        from views.wgpu_ui import _components_dir, _starter_components
        out = []
        for i, (name, _faces, groups, kept) in enumerate(self._model(ctx)):
            linked = bool(groups) and all(_is_linked(g) for g in groups)
            verbs = ["apply", "edit", "select", "replace", "-", "rename",
                     "unkeepfile" if kept is not None else "keepfile", "-", "delete"]
            if not groups:
                verbs = ["apply", "rename", "-", "unkeepfile"]
            out.append({"key": f"cm:{i}", "name": name or "Component", "source": "linked" if linked else "model",
                        "category": "" if groups else "Kept, not placed", "uses": len(groups), "hex": "",
                        "thumb": None, "verbs": tuple(verbs)})
        res = _components_dir()
        for key, e in _starter_components().items():
            people = key.startswith("pp:")
            thumb = res / ("%s.png" % e["key"] if people else "thumbs/%s.png" % e["key"])
            out.append({"key": key, "name": e["name"], "source": "bundled",
                        "category": "People (2D)" if people else "Starters (3D)", "uses": 0, "hex": "",
                        "thumb": str(thumb) if thumb.exists() else None, "verbs": ("apply",)})
        return out

    def _row(self, ctx, item):
        try:
            return self._model(ctx)[int(item["key"].partition(":")[2])]
        except (ValueError, IndexError):
            return None

    def type_actions(self, ctx):
        n = sum(1 for _n, _f, gs, kept in self._model(ctx) if kept is not None and not gs)
        return [("purge", f"Stop keeping unplaced components ({n})" if n else "Stop keeping unplaced components (none)")]

    def type_action(self, ctx, action):
        if action == "purge":
            from core.history import CompoundCommand, KeepComponentCommand
            idle = [kept for _n, _f, gs, kept in self._model(ctx) if kept is not None and not gs]
            if idle:
                ctx.vp.history.execute(CompoundCommand([KeepComponentCommand(t, False) for t in idle]))
                ctx.vp.notify_scene_changed()
            ctx.flash(f"{len(idle)} unplaced components dropped" if idle else "Every kept component is placed")

    def run(self, ctx, verb, item, arg=""):
        from views.wgpu_ui import _library_pick, _select
        key = item["key"]
        if not key.startswith("cm:"):
            if verb == "apply":
                _library_pick(ctx.win, ctx.view, key)
            return
        row = self._row(ctx, item)
        if row is None:
            return
        name, _f, groups, kept = row
        definition = kept if kept is not None else (groups[0] if groups else None)
        if definition is None:
            return
        if verb == "apply":
            from core.kept_components import make_template, place_copy
            ctx.win._start_place(place_copy(kept if kept is not None else make_template(definition)))
        elif verb == "edit" and groups:
            g = next((g for g in groups if g in ctx.scene.groups), None)
            if g is None:
                ctx.flash("Its copies are nested: open their group first")
                return
            _select(ctx.win, [g])
            ctx.vp.begin_group_edit(g)
        elif verb == "select":
            from ingetrazo_render import select_component_instances
            select_component_instances(ctx.win, groups)
        elif verb == "replace":
            from core.group import Group
            swap = [g for g in ctx.scene.selection
                    if isinstance(g, Group) and g.is_component() and g.mesh is not definition.mesh]
            if swap:
                ctx.win.replace_selection_with(definition)
            else:
                ctx.flash("Select copies of another component first")
        elif verb == "rename" and arg.strip():
            ctx.core("rename_component", placement=definition.uid, name=arg.strip())
        elif verb in ("keepfile", "unkeepfile"):
            ctx.win.keep_component(definition, verb == "keepfile")
        elif verb == "delete":
            n = len(groups)
            _ask(ctx.view, ("delete", self.id, key), f"Delete every copy of “{name}”?",
                 f"{n} cop{'ies' if n != 1 else 'y'} and their geometry go (Undo brings them back).", "Delete")

    def confirmed(self, ctx, what, key, text=""):
        row = self._row(ctx, {"key": key})
        if what == "delete" and row is not None:
            uids = [g.uid for g in row[2]]
            try:
                ctx.core("delete_placements", placements=uids)
            except KeyError:
                ctx.flash("Some copies are nested in other groups: delete those from the tree")
            ctx.vp.update()

    def inspector(self, ctx, item):
        if not item["key"].startswith("cm:"):
            return None
        row = self._row(ctx, item)
        if row is None:
            return None
        name, faces, groups, kept = row
        rows = [("Copies", str(len(groups))), ("Faces", f"{faces:,}".replace(",", " ")),
                ("Kept in file", "yes" if kept is not None else "no")]
        acts = [(f"lib:apply:{item['key']}", "Place a copy"), (f"lib:select:{item['key']}", "Select copies"),
                (f"lib:edit:{item['key']}", "Open for editing")]
        return name or "Component", [("Component in this model", rows), ("Actions", acts)]


class ProfilesProvider(Provider):
    """Coming: section outlines (steel shapes, extrusion and sweep
    profiles) to draw, extrude and run along a path."""
    id, label, ready = "profiles", "Profiles", False


def _is_linked(g) -> bool:
    try:
        from core import links
        return links.is_linked(g)
    except Exception:   # noqa: BLE001 - a missing link module means "no"
        return False


def _catalogue():
    from core.i18n import current_language
    from core.paths import app_root
    return _bundled_by_key(app_root() / "resources", current_language())


@lru_cache(maxsize=4)
def _bundled_by_key(resources, language):
    from ingetrazo_core import bundled_materials
    return {item["key"]: item for item in json.loads(bundled_materials(resources, language))}


def _pick_paint(win, *, name="", recipe=None):
    from ingetrazo_core import pick_library_paint
    from tools.paint import PaintTool
    if pick_library_paint(win.viewport.scene, PaintTool, name=name, recipe=recipe):
        win._activate_tool("paint")


def _rename_paint(scene, old, new) -> None:
    from tools.paint import PaintTool
    mat = getattr(PaintTool, "current_material", None)
    if mat is not None and mat.name == old:
        from ingetrazo_core import pick_library_paint
        pick_library_paint(scene, PaintTool, name=new)


#: The registry, in tab order.
PROVIDERS = [MaterialsProvider(), ComponentsProvider(), ProfilesProvider()]


def provider(type_id):
    return next((p for p in PROVIDERS if p.id == type_id), PROVIDERS[0])


# ---- state ------------------------------------------------------------------------

def _state(view) -> dict:
    st = view.__dict__.get("_ui_lib")
    if st is None:
        saved = _settings_json("library/state", {})
        st = {"type": saved.get("type", "materials"), "view": int(saved.get("view", 0)),
              "sort": saved.get("sort", "name"), "filter": "", "fold": set(saved.get("fold", [])),
              "open": set(saved.get("open", [])), "recent": _settings_json("library/recent", []), "mine_rev": 0,
              "cursor": None, "sig": None, "items": {}, "scope": saved.get("scope", "s:model")}
        if st["sort"] not in dict(SORTS):
            st["sort"] = "name"
        st["facet"] = saved.get("facet", "all")
        if st["type"] != "registry" and st["type"] not in [p.id for p in PROVIDERS if p.ready]:
            st["type"] = "materials"      # "colours" was a type of its own before 2026-11-22
        view._ui_lib = st
    return st


def _save_state(st) -> None:
    _store_json("library/state", {"type": st["type"], "view": st["view"], "sort": st["sort"],
                                  "facet": st.get("facet", "all"), "scope": st.get("scope", "s:model"),
                                  "fold": sorted(st["fold"]), "open": sorted(st.get("open", ()))})


# ---- what the sheet lists (library_view.rs) ---------------------------------------------
def _browse(st) -> str:
    return json.dumps({"sort": st.get("sort", "name"), "facet": st.get("facet", "all"), "filter": st.get("filter", ""),
                       "scope": st.get("scope") or "s:model", "fold": sorted(st.get("fold", ())),
                       "recent": list(st.get("recent", ())), "cursor": st.get("cursor")})


def filtered(items, st):
    """The items the facet and the search keep."""
    by_key = {e["key"]: e for e in items}
    return [by_key[k] for k in library_filtered(json.dumps(items, default=str), _browse(st))]


def side_rows(items, st) -> list:
    """The side list: (id, label, count, depth, kind, fold) per row."""
    return library_side(json.dumps(items, default=str), _browse(st))


def groups_for(items, st):
    """The content: [(caption, sub, items)]."""
    by_key = {e["key"]: e for e in items}
    return [(cap, sub, [by_key[k] for k in keys])
            for cap, sub, keys in library_groups(json.dumps(items, default=str), _browse(st))]


def _thumb(view, path) -> str:
    """A texture or thumbnail as a chrome icon, sharp on a Retina tile."""
    if not path:
        return ""
    key = "lib:" + str(path)
    sent = view.__dict__.setdefault("_ui_tex_icons", set())
    if key not in sent:
        sent.add(key)
        from ingetrazo_render import swatch_pixels
        px = THUMB_PX * 2
        rgba = swatch_pixels(path, px, True)
        if rgba is not None:
            view.renderer.ui_set_icons([(key, px, px, rgba)])
    return key


def item(view, key):
    return _state(view)["items"].get(key)


# ---- the sheet -------------------------------------------------------------------------

def _chrome_sheet():
    from ingetrazo_render import chrome_sheet
    return chrome_sheet


class LibrarySheet(_chrome_sheet().Provider):
    """The Library on the large sheet (option B of the 2026-11-23 review):
    types as tabs, scopes and categories on the side (the linked files'
    materials read-only), 96-pt tiles, the focused item's details and
    model usage on the right. The inspector edits the chosen material."""
    id = "library"

    def __init__(self, view, paint_target=None):
        self.view = view
        self.paint_target = paint_target
        if paint_target is not None:
            self.id = "tree-material"
        self.catalogue = self.registry = None

    def signature(self, ctx):
        win = ctx.win
        scene = win.viewport.scene
        from views.wgpu_ui import _current_material_name
        st = _state(self.view)
        return (id(scene), scene.content_version, _current_material_name(), st["mine_rev"], tuple(st["recent"][:12]))

    def _items(self, ctx):
        st = _state(self.view)
        lctx = Ctx(ctx.win, self.view)
        counts, active = {}, []
        for p in PROVIDERS:
            if not p.ready:
                continue
            items = p.items(lctx)
            counts[p.id] = sum(1 for e in items if e["source"] in ("model", "linked"))
            if p.id == st["type"]:
                active = items
        st["items"] = {e["key"]: e for e in active}
        return active, counts

    def build(self, ctx):
        cs = _chrome_sheet()
        from views.wgpu_ui import _current_material_name
        view, st = self.view, _state(self.view)
        prov = provider(st["type"])
        items, counts = self._items(ctx)
        target = self.paint_target
        s = cs.Sheet(self.id, "Choose material" if target else "", width=1240)
        if target is None:
            for p in PROVIDERS:
                s.tab(p.id, p.label, counts.get(p.id, "") if p.ready else "soon", on=p.id == st["type"], enabled=p.ready)
            s.tab("registry", "Project registry", on=st["type"] == "registry")
        if st["type"] == "registry":
            from ingetrazo_render import RegistryEditor
            if self.registry is None:
                self.registry = RegistryEditor(ctx.win.viewport.scene)
            self.registry.build(s)
            return s
        scene = ctx.win.viewport.scene
        browse = json.loads(_browse(st))
        if target:
            target.prepare(items, browse)
            st["items"] = {e["key"]: e for e in items}
        current = target.current if target else _current_material_name()
        out = json.loads(library_sheet(json.dumps({
            "type": prov.id, "items": items, "st": browse,
            "current": current if prov.id == "materials" else None,
            "model_uses": dict(material_uses(scene)) if prov.id == "materials" else {},
            "unnamed": len(unnamed_colours_cached(scene)) if prov.id == "materials" else 0}, default=str)))
        for path in out["thumbs"]:
            _thumb(view, path)
        st["scope"], searching, online = out["scope"], out["searching"], out["online"]
        s.search(out["search"])
        s.menus.append(out["sort"])
        s.side = out["side"]
        if prov.id == "components":
            from core.i18n import tr
            s.side_item("s:online", tr("Component library"), on=online)
        if online or (prov.id == "components" and searching):
            from views.component_catalogue import CatalogueSession
            if self.catalogue is None:
                self.catalogue = CatalogueSession(ctx)
            if not searching:
                return self.catalogue.build(s, "")
        else:
            self.close_catalogue()
        c = s.content(**out["content"])
        for g in out["groups"]:
            if g["caption"]:
                c.caption(g["caption"], right=g["right"])
            c.tiles(g["tiles"], size=96)
        if target is None and out["type_actions"]:
            acts = [cs.verb(f"typeact:{a}", lab, quiet=True) for a, lab in prov.type_actions(Ctx(ctx.win, view))]
            if acts:
                c.buttons(acts, align="left")
        st["cursor"] = out["cursor"]
        d = s.detail(width=340, empty="Pick an item to see it here.")
        if target:
            d.fields(target, "tree-paint")
            d.buttons([cs.verb("custompaint", "Apply colour"), cs.verb("activepaint", "Use active paint"),
                       cs.verb("clearpaint", "Remove paint")])
            if st.get("cursor"):
                d.buttons([cs.verb("apply", "Apply material", primary=True)], align="fill")
        else:
            d.blocks = out["detail"]
        e = st["items"].get(st["cursor"]) if st["cursor"] else None
        if e is not None and prov.id == "components":
            self._component_detail(d, prov, ctx, e)
        s.footer((), "Click to see it · double-click or ↩ uses it · esc back to the model")
        if st.get("cursor"):
            s.focus(st["cursor"], force=bool(st.pop("force_focus", False)))
        return self.catalogue.build(s, st["filter"]) if self.catalogue is not None else s

    def _component_detail(self, d, prov, ctx, e):
        cs = _chrome_sheet()
        lctx = Ctx(ctx.win, self.view)
        got = prov.inspector(lctx, e)
        if got is not None:
            _title, sections = got
            for title, rows in sections:
                if title == "Actions":
                    continue
                d.kv(rows)
        row = prov._row(lctx, e) if e["key"].startswith("cm:") else None
        if row:
            group = row[3] or next(iter(row[2]), None)
            if record := (getattr(group, "ext", None) or {}).get("catalogue"):
                from core.i18n import current_language, tr
                from ingetrazo_render import ComponentCatalogue
                d.text(ComponentCatalogue.credit_line(json.dumps(record), current_language(), tr))
        verbs = [cs.verb("apply", "Place a copy", key="↩", primary=True)]
        if e["key"].startswith("cm:"):
            verbs += [cs.verb("select", "Select copies"), cs.verb("edit", "Open for editing")]
        d.buttons(verbs, align="fill")
        if e["key"].startswith("cm:"):
            more = [v for v in ("rename", "keepfile", "unkeepfile", "delete") if v in e["verbs"]]
            d.buttons([cs.verb(v, {"rename": "Rename…"}.get(v, VERBS[v]), quiet=True) for v in more], align="left")

    def _rename(self, ctx, e):
        from views import chrome_forms
        prov = provider(_state(self.view)["type"])
        lctx = Ctx(ctx.win, self.view)
        title = "Rename material" if prov.id == "materials" else "Rename component"

        def done(text):
            if text and text.strip() and text.strip() != e["name"]:
                try:
                    prov.run(lctx, "rename", e, text.strip())
                except (KeyError, ValueError) as exc:
                    lctx.flash(str(exc).strip("'\""))
                if prov.id == "materials":
                    _state(self.view)["cursor"] = f"m:{text.strip()}"
                    _state(self.view)["force_focus"] = True
            refresh(ctx.win)
        chrome_forms.ask_text(ctx.win, title, "Name:", text=e["name"], done=done)

    # -- what comes back ---------------------------------------------------------------
    def act(self, ctx, verb, args):
        view, win = self.view, ctx.win
        st = _state(view)
        arg = args[0] if args else ""
        lctx = Ctx(win, view)
        if self.paint_target is not None:
            if verb == "tab":
                return
            if verb == "verb" and arg in ("custompaint", "activepaint", "clearpaint"):
                self.paint_target.apply(win.viewport, arg)
                _chrome_sheet().close(win)
                return
        if self.catalogue is not None:
            if verb == "tab" or (verb == "side" and arg != "s:online" and not arg.startswith("online-cat:")):
                self.close_catalogue()
            elif verb == "side":
                st["scope"] = "s:online"
                self.catalogue.act(verb, args)
                _save_state(st)
                return
            elif self.catalogue.act(verb, args):
                return
        if verb == "verb" and self.registry is not None and st["type"] == "registry":
            path = ""
            if arg == "registry_csv":
                from views.filedialogs import file_dialogs
                path, _ = file_dialogs.getOpenFileName(win, "Codes CSV", "", "CSV (*.csv)")
            if self.registry.act(win, arg, path) and arg == "registry_apply":
                _chrome_sheet().close(win)
            return
        if verb == "search":
            st["filter"] = arg
        elif verb == "tab":
            if arg == "registry" or any(p.id == arg and p.ready for p in PROVIDERS):
                st["type"], st["scope"], st["cursor"] = arg, "s:model", None
                _save_state(st)
        elif verb == "side":
            st["scope"], st["cursor"] = arg, None
            _save_state(st)
        elif verb == "fold":
            st["fold"].symmetric_difference_update({arg})
            _save_state(st)
        elif verb == "headmenu" and len(args) > 1:
            opt = args[1]
            if opt.startswith("facet:"):
                st["facet"] = opt[6:] if opt[6:] in ("all", "colour", "image") else "all"
            elif opt in dict(SORTS):
                st["sort"] = opt
            _save_state(st)
        elif verb == "focus":
            st["cursor"] = arg
        elif verb == "open":
            self.run(ctx, "apply", arg)
        elif verb == "menu" and len(args) > 1:
            self.run(ctx, args[1], arg)
        elif verb == "verb":
            if arg.startswith("typeact:"):
                provider(st["type"]).type_action(lctx, arg[len("typeact:"):])
            elif st.get("cursor"):
                self.run(ctx, arg, st["cursor"])

    def run(self, ctx, verb, key):
        view, win = self.view, ctx.win
        st = _state(view)
        e = st["items"].get(key)
        if e is None:
            return
        prov = provider(st["type"])
        lctx = Ctx(win, view)
        if self.paint_target is not None:
            if verb == "apply":
                name, recipe = prov.paint_recipe(lctx, e)
                self.paint_target.apply(win.viewport, f"m:{name}" if name else key, recipe)
                _chrome_sheet().close(win)
            return
        if verb == "rename":
            self._rename(ctx, e)
            return
        if verb == "opensource":
            path = e.get("file")
            if path and hasattr(win, "open_path"):
                win.open_path(path)
            return
        if verb == "apply":
            _remember(view, key)
            st["cursor"] = key
        prov.run(lctx, verb, e)
        if verb == "duplicate":
            st["force_focus"] = True
        if verb == "apply" or (verb == "edit" and prov.id == "materials"):
            # Painting, placing and the material inspector use the model workspace.
            _chrome_sheet().close(win)

    def close_catalogue(self):
        if self.catalogue is not None:
            self.catalogue.close()
            self.catalogue = None

    def closed(self, ctx):
        self.registry = None
        self.close_catalogue()

def refresh(win) -> None:
    _chrome_sheet().refresh(win)


def _sheet_provider(view):
    p = view.__dict__.get("_ui_lib_sheet")
    if p is None:
        p = view._ui_lib_sheet = LibrarySheet(view)
    return p


def before_frame(view, win) -> None:
    """The Paint strip (C's palette): the model's materials and the recent
    ones, the current one's name, Browse all… (the sheet)."""
    _palette(view, win)


# ---- opening ---------------------------------------------------------------------------

def _land(view, win) -> None:
    """Which type opens: a component being placed opens Components, Paint
    opens Materials."""
    st = _state(view)
    tool = getattr(win.viewport, "active_tool", None) or getattr(getattr(win.viewport, "tools", None), "active", None)
    if type(tool).__name__ == "PlaceGroupTool":
        st["type"] = "components"
    elif _active_tool(win) == "paint":
        st["type"] = "materials"


def _active_tool(win) -> str:
    return next((k for k, a in getattr(win, "_tool_actions", {}).items() if a.isChecked()), "")


def open_library(win, type_id=None, search=None, scope=None, *, paint_target=None) -> bool:
    """Window › Library › <type>, ⌘K, the rail, the Paint strip's Browse
    all…: the Library sheet on that type. Requires an active document view."""
    view = getattr(getattr(win, "_doc", None), "stand_in", None)
    if view is None:
        return False
    st = _state(view)
    _land(view, win)
    if type_id:
        st["type"] = type_id
    if scope:
        st["scope"] = scope
    st["filter"] = search or ""
    sheet = LibrarySheet(view, paint_target) if paint_target is not None else _sheet_provider(view)
    return _chrome_sheet().open(win, sheet, search=search)


def set_mode(view, win, mode) -> None:
    """Route existing Project commands to their lens or Library sheet."""
    if mode == "library":
        open_library(win)
        return
    if mode in ("problems", "deliveries"):
        from views.wgpu_ui import apply_dock_width
        view._ui_lens = 2
        apply_dock_width(view)
        mode = ""
    view._ui_mode = mode
    view._ui_mat_focus = None
    view._ui_rules_cache = None
    view._ui_tree_sig = None
    view._ui_insp_sig = None
    view.update()


def install(win) -> None:
    """Window › Library › Materials / Components: menu entries (so ⌘K
    finds them) and the rail's Components button."""
    if getattr(win, "_lib_installed", False):
        return
    win._lib_installed = True
    if "_command_store" in vars(win):
        from ingetrazo_render import shell_action as ShellAction
        win._lib_actions = {p.id: ShellAction(win, p.label, run=lambda t=p.id: open_library(win, t), listed=False)
                            for p in PROVIDERS if p.ready}
        win._act_library_components = ShellAction(win, "Components", run=lambda: open_library(win, "components"), listed=False)
        return
    from PySide6.QtGui import QAction
    menu = None
    for a in win.menuBar().actions():
        if a.menu() is not None and a.text().replace("&", "") in ("Window", "Ventana"):
            menu = a.menu()
            break
    from PySide6.QtCore import QObject
    # The native shell's window is not a QObject: the actions then have no
    # parent and are kept alive on the window instead.
    owner = win if isinstance(win, QObject) else None
    acts = {}
    for p in PROVIDERS:
        if not p.ready:
            continue
        act = QAction(p.label, owner)
        act.triggered.connect(lambda _=False, t=p.id: open_library(win, t))
        acts[p.id] = act
    if menu is not None:
        sub = menu.addMenu("Library")
        for act in acts.values():
            sub.addAction(act)
    comp = QAction("Components", owner)
    comp.setToolTip("Components: the Library's components, to place")
    comp.triggered.connect(lambda: open_library(win, "components"))
    win._act_library_components = comp
    win._lib_actions = acts


# ---- the palette strip (Paint's fast path) --------------------------------------------

#: How many swatches the strip shows: the model's, then recent ones not in it.
STRIP_MODEL = 10
STRIP_RECENT = 5


def palette_items(view, win) -> list:
    """(id, name, hex, icon, active) per swatch; ("-", …) a rule; ("", name…)
    the current material's name."""
    from views.wgpu_ui import _current_material_name, _hex
    scene = win.viewport.scene
    cur = _current_material_name()
    out = []
    mats = list((getattr(scene, "materials", None) or {}).items())
    uses = material_uses(scene)
    # The model's, most used first (what you paint with most is under the hand).
    mats = [mats[i] for i in library_by_use([n for n, _m in mats], [uses.get(n, 0) for n, _m in mats])]
    for name, mat in mats[:STRIP_MODEL]:
        tex = getattr(mat, "texture", None)
        icon = _thumb(view, tex["path"]) if tex and tex.get("path") else ""
        out.append((f"m:{name}", name, _hex(mat.color), icon, name == cur))
    st = _state(view)
    from ingetrazo_core import recent_swatches
    from tools.paint import PaintTool
    rec = []
    for key in st["recent"]:
        if len(rec) >= STRIP_RECENT:
            break
        kind = key.partition(":")[0]
        if kind not in ("lib", "ral", "my"):
            continue
        hexc, icon, name = _recent_look(key)
        if name and name not in dict(mats) and all(row[1] != name for row in rec):
            rec.append((key, name, hexc, _thumb(view, icon) if icon else "", name == cur))
    for key, name, colour, path in recent_swatches(PaintTool):
        if len(rec) >= STRIP_RECENT:
            break
        if all(row[1] != name for row in out + rec):
            rec.append((key, name, colour, _thumb(view, path) if path else "", name == cur))
    if rec:
        out.append(("-", "", "", "", False))
        out += rec
    if cur:
        out.append(("-", "", "", "", False))
        out.append(("", cur, "", "", False))
    return out


def _recent_look(key):
    """(hex, image path, name) of a bundled or kept material, from its key."""
    kind, _, rest = key.partition(":")
    try:
        if kind in ("ral", "lib"):
            item = _catalogue().get(key)
            if item:
                return item["hex"], item["thumb"], item["name"]
        elif kind == "my":
            e = _settings_json(_MINE_KEY, [])[int(rest)]
            tex = (e.get("texture") or {}).get("path")
            return _hex(e.get("color")), tex, e.get("name", "")
    except (OSError, ValueError, IndexError, KeyError):
        pass
    return "", None, ""


def _palette(view, win) -> None:
    from views.wgpu_ui import _active_tool_id
    items = palette_items(view, win) if _active_tool_id(win) == "paint" else []
    if items != getattr(view, "_ui_palette", None):
        view._ui_palette = items
        view.renderer.ui_set_palette(items)


def paint_from_strip(win, key) -> None:
    """A swatch of the strip: paint with it (a recent bundled one too)."""
    view = getattr(getattr(win, "_doc", None), "stand_in", None)
    if key.startswith("stamp:"):
        from ingetrazo_core import pick_recent
        from tools.paint import PaintTool
        pick_recent(win.viewport.scene, PaintTool, json.loads(key[6:]))
        win._activate_tool("paint")
        return
    if view is not None:
        _remember(view, key)
    from views.wgpu_ui import _paint_with
    if key.startswith("my:") and view is not None:
        e = _settings_json(_MINE_KEY, [])
        try:
            entry = e[int(key[3:])]
        except (ValueError, IndexError):
            return
        provider("materials").run(Ctx(win, view), "apply", {"key": key, "name": entry.get("name", "")})
        return
    _paint_with(win, key)


# ---- actions ------------------------------------------------------------------

def _ask(view, pending, title, body, yes) -> None:
    """A question over the sheet (or the window): Delete… asks first."""
    from views import chrome_dialogs
    win = view.window() if hasattr(view, "window") else None

    def done(answer):
        if answer == "yes":
            confirmed(win, view, pending)
            refresh(win)
    chrome_dialogs.ask_later(win, title, body, [("cancel", "Cancel"), ("yes", yes)], done,
                             default="yes", cancel="cancel", tone="warning")


def confirmed(win, view, pending, text="") -> None:
    """A Library question answered yes."""
    what, type_id, key = pending
    p = provider(type_id)
    if hasattr(p, "confirmed"):
        p.confirmed(Ctx(win, view), what, key, text)
    _state(view)["sig"] = None


def _remember(view, key) -> None:
    st = _state(view)
    st["recent"] = [key] + [k for k in st["recent"] if k != key][: RECENT_MAX - 1]
    _store_json("library/recent", st["recent"])


def action(win, view, arg) -> None:
    """``verb:key`` from an inspector chip (the material being edited in
    the dock's inspector, C's path)."""
    if view is None:
        return
    if arg == "done":
        view._ui_mat_focus = view._ui_insp_sig = None
        view.update()
        return
    if SEP not in arg and ":" in arg:
        verb, _, rest = arg.partition(":")
        arg = verb + SEP + rest
    verb, _, rest = arg.partition(SEP)
    key, _, extra = rest.partition(SEP)
    if verb in ("map", "unmap") and not extra:
        key, _, extra = key.rpartition(":")
    st = _state(view)
    ctx = Ctx(win, view)
    try:
        if verb == "browse":
            open_library(win, key or None)
            return
        it = _item_anywhere(view, win, key)
        if it is None:
            return
        p = provider("components" if key.startswith(("cm:", "sc:", "pp:")) else "materials")
        if verb == "apply":
            _remember(view, key)
        p.run(ctx, verb, it, extra)
        view._ui_insp_sig = None
    except (KeyError, ValueError) as exc:
        ctx.flash(str(exc).strip("'\""))
    view.update()


def _item_anywhere(view, win, key):
    """The item for ``key``, from the last sheet build or the model."""
    it = item(view, key)
    if it is not None:
        return it
    if key.startswith("m:"):
        name = key[2:]
        mat = win.viewport.scene.materials.get(name)
        if mat is None:
            return None
        return {"key": key, "name": name, "source": "model", "category": "",
                "uses": material_uses(win.viewport.scene).get(name, 0), "hex": _hex(mat.color),
                "thumb": (mat.texture or {}).get("path"), "verbs": ()}
    return None


# ---- inspector (C: the dock's inspector edits the material) ----------------------

def inspector_key(view, win):
    """The material the dock's inspector edits: one picked for editing
    (a swatch's Edit…), else Paint's current one while Paint is active."""
    from views.wgpu_ui import _active_tool_id, _current_material_name
    key = view.__dict__.get("_ui_mat_focus")
    if key and key.startswith("m:") and key[2:] in win.viewport.scene.materials:
        return key
    if _active_tool_id(win) == "paint":
        cur = _current_material_name()
        if cur and cur in win.viewport.scene.materials:
            return f"m:{cur}"
        from tools.paint import PaintTool
        if not PaintTool.current_is_default:
            return "brush:"
    return None


def inspector(view, win):
    """``(title, sections)`` for the material the inspector edits, or None
    (the inspector then shows the selection as usual)."""
    key = inspector_key(view, win)
    if key == "brush:":
        from ingetrazo_core import brush_inspector
        from tools.paint import PaintTool
        return brush_inspector(PaintTool)
    it = _item_anywhere(view, win, key) if key else None
    if it is None:
        return None
    got = provider("materials").inspector(Ctx(win, view), it)
    if got is None:
        return None
    return got


def edit_field(win, view, arg, text) -> None:
    """An inspector field of a material: ``<field>:<material name>``."""
    field, _, name = arg.partition(":")
    it = _item_anywhere(view, win, f"m:{name}") or {"key": f"m:{name}", "name": name}
    provider("materials").edit(Ctx(win, view), it, field, text)
    from ingetrazo_core import refresh_library_paint
    from tools.paint import PaintTool
    refresh_library_paint(win.viewport.scene, PaintTool)
    refresh(win)
