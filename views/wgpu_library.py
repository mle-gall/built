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

and runs a verb on one (``run``). This module owns the browsing state
(type, grid/list, sort, filter, folded sections, recent picks), turns the
items into the rows ``ui.rs`` draws (``Ui::set_library``), and answers the
``lib:<verb>\\x1f<key>[\\x1f<arg>]`` actions it sends back. Model edits go
through ``core.protocol.commands`` (docs/core-protocol.md §1).

The dock's places: Project (the tree, with Problems as one of its modes)
and Library, switched in the dock header. Opening the Library remembers
where Project was; the switch or Esc goes back there."""
from __future__ import annotations

import json
import re
from collections import Counter

from PySide6.QtCore import QSettings, Qt
from PySide6.QtGui import QImage

SEP = "\x1f"
#: Sections, in the order they show.
SECTIONS = [("model", "In this model"), ("linked", "From linked files"), ("mine", "My library"),
            ("bundled", "Bundled")]
SORTS = [("name", "Name"), ("used", "Most used"), ("recent", "Recently used")]
#: Verb ids and their menu labels (a provider may relabel "apply").
VERBS = {"apply": "Use", "edit": "Edit in the inspector", "duplicate": "Duplicate", "rename": "Rename",
         "keep": "Add to My library", "unkeep": "Remove from My library", "select": "Select uses",
         "replace": "Replace selection with this", "keepfile": "Keep in file", "unkeepfile": "Stop keeping in file",
         "delete": "Delete…", "-": ""}
RECENT_MAX = 40
ROW_CAP = 600
THUMB_PX = 96
_MINE_KEY = "library/mine"


def _natural(text):
    return [int(p) if p.isdigit() else p for p in re.split(r"(\d+)", (text or "").lower())]


def _hex(rgb) -> str:
    return "#%02x%02x%02x" % tuple(int(max(0.0, min(1.0, float(c))) * 255) for c in rgb[:3]) if rgb else ""


def _parse_hex(text):
    t = (text or "").strip().lstrip("#")
    if len(t) != 6:
        raise ValueError("A colour is six hex digits, like #a0703c")
    v = int(t, 16)
    return ((v >> 16 & 255) / 255.0, (v >> 8 & 255) / 255.0, (v & 255) / 255.0)


def _pretty(stem: str) -> str:
    """A bundled file's name for people: ``brick_red`` → ``Brick red``."""
    text = str(stem).replace("_", " ").replace("-", " ").strip()
    return text[:1].upper() + text[1:]


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
    from core.group import iter_placements
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
        for g, _m in iter_placements(top):
            paint = getattr(g, "material", None) or {}
            if paint.get("mat"):
                uses[paint["mat"]] += 1
    scene._lib_uses = (sig, uses)
    return uses


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
    from core.group import iter_placements
    scene, memo = ctx.scene, {}
    picked = [f for f in scene.mesh.faces
              if f.attrs.get("mat") == name or (f.attrs.get("back") or {}).get("mat") == name]
    for g in scene.groups:
        for pg, _m in iter_placements(g):
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
    apply_label = "Use"
    empty = ""

    def items(self, ctx) -> list[dict]:
        return []

    def verbs(self) -> list[tuple[str, str]]:
        return [(k, self.apply_label if k == "apply" else v) for k, v in VERBS.items() if k != "-"]

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


#: Inspector encodings ui.rs draws (inspector_rows): a field, a switch, a swatch.
_ED = "\x1d"


def _choice(id_, options, at) -> str:
    return f"{_ED}choice{_ED}{id_}{_ED}{'|'.join(options)}{_ED}{at}"


def _swatch(id_, hexc) -> str:
    return f"{_ED}color{_ED}{id_}{_ED}{hexc}"


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
    apply_label = "Paint with it"
    empty = "No materials yet."
    #: Browsing by base, offered in the sort menu.
    facets = [("facet:all", "Every base"), ("facet:colour", "Colour bases"), ("facet:image", "Image bases")]

    def verbs(self):
        return super().verbs() + [("name", "Name these colours")]

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
        from views import tray
        out = []
        manifest = tray._TEX_DIR / "library.json"
        if manifest.exists():
            data = json.loads(manifest.read_text(encoding="utf-8"))
            for cat in data.get("categories", []):
                title = tray.MaterialsPanel.CATEGORY_NAMES.get(cat["id"], cat["id"])
                for i, it in enumerate(cat.get("items", [])):
                    out.append({"key": f"lib:{cat['id']}:{i}", "name": it["name"], "label": _pretty(it["name"]),
                                "source": "bundled", "category": title, "base": "image", "uses": 0, "hex": "",
                                "thumb": str(tray._TEX_DIR / "library" / it["file"]), "verbs": ("apply",)})
        if tray._RAL_FILE.exists():
            data = json.loads(tray._RAL_FILE.read_text(encoding="utf-8"))
            for fam in data.get("families", []):
                for c in fam.get("colors", []):
                    rgb = [v / 255.0 if max(c["rgb"]) > 1 else v for v in c["rgb"]]
                    out.append({"key": f"ral:{c['code']}", "name": "%s %s" % (c["code"], tray._ral_name(c)),
                                "short": c["code"],
                                "source": "bundled", "category": "Colours (RAL)", "base": "colour", "uses": 0,
                                "hex": _hex(rgb), "thumb": None, "verbs": ("apply",)})
        return out

    def items(self, ctx):
        if getattr(self, "_bundled_cache", None) is None:
            self._bundled_cache = self._bundled(ctx)
        return self._model_items(ctx) + self._mine_items(ctx) + [dict(e) for e in self._bundled_cache]

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

    def run(self, ctx, verb, item, arg=""):
        key, name = item["key"], item["name"]
        kind = key.partition(":")[0]
        if verb == "name":
            self.type_action(ctx, "name")
        elif kind == "uc":
            entry = unnamed_colours_cached(ctx.scene).get(key[3:])
            if entry is None:
                return
            if verb == "apply":
                panel = _materials_panel(ctx.win)
                if panel is not None:
                    panel._apply_color(entry["color"])   # paints as a named material
            elif verb == "select":
                loose = set(map(id, ctx.scene.mesh.faces))
                faces = [f for f, side in entry["faces"] if side == "front" and id(f) in loose]
                ctx.scene.select(faces + [g for g in entry["groups"] if g in ctx.scene.groups])
                getattr(ctx.vp, "notify_scene_changed", ctx.vp.update)()
        elif verb == "apply":
            if kind == "my":
                e = self._mine_entry(item)
                panel = _materials_panel(ctx.win)
                if panel is None:
                    return
                tex = e.get("texture")
                if tex and tex.get("path"):
                    panel._apply_texture(tex["path"], sw=tex.get("sw"), sh=tex.get("sh"), name=name,
                                         opacity=e.get("opacity"))
                else:
                    panel._apply_color(tuple(e.get("color") or (1, 1, 1)), name=name)
            else:
                from views.wgpu_ui import _paint_with
                _paint_with(ctx.win, key)
        elif verb == "edit" and kind == "m":
            ctx.view._ui_mat_focus = key
        elif verb == "select" and kind == "m":
            n = select_material_uses(ctx, name)
            ctx.flash(f"{n} selected wearing {name}" if n else f"Nothing in the open context wears {name}")
        elif verb == "duplicate" and kind == "m":
            out = ctx.core("duplicate_material", name=name)
            if out and out.get("name"):
                ctx.view._ui_mat_focus = f"m:{out['name']}"
                _state(ctx.view)["cursor"] = f"m:{out['name']}"
        elif verb == "rename" and kind == "m" and arg.strip() and arg.strip() != name:
            ctx.core("rename_material", name=name, new_name=arg.strip())
            _rename_paint(name, arg.strip())
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
                mine.append({"type": self.id, "name": name, "color": list(mat.color) if mat.color else None,
                             "texture": dict(mat.texture) if mat.texture else None, "opacity": mat.opacity})
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
        from views.wgpu_ui import _edit
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
        name = mat.name
        tex = mat.texture or {}
        image = bool(tex.get("path"))
        rows = [("Name", _edit(f"libmat:name:{name}", name)),
                ("Base", _choice(f"libmat:base:{name}", ["Colour", "Image"], 1 if image else 0))]
        acts = [(f"lib:apply:{key}", "Paint with it"), (f"lib:select:{key}", "Select uses")]
        if image:
            rows += [("Image", _file_stem(tex["path"])),
                     ("Tile (m)", _edit(f"libmat:tile:{name}",
                                        f"{tex.get('sw', 1):g} × {tex.get('sh', tex.get('sw', 1)):g}"))]
            acts.append((f"lib:image:{key}", "Other image…"))
        else:
            rows.append(("Colour", _swatch(f"libmat:color:{name}", _hex(mat.color) or "#cccccc")))
        pbr = mat.pbr or {}
        surface = []
        for channel, label, map_key in PBR_ROWS:
            value = pbr.get(channel)
            surface.append((label, _edit(f"libmat:{channel}:{name}", "" if value is None else f"{float(value):g}")))
            if pbr.get(map_key):
                surface.append((f"{label} map", _file_stem(pbr[map_key].get("path", ""))))
        maps = [(f"lib:map:{key}:{m}" if not pbr.get(m) else f"lib:unmap:{key}:{m}",
                 (f"{label} map…" if not pbr.get(m) else f"Clear {label.lower()} map"))
                for _c, label, m in PBR_ROWS]
        opacity = 100 if mat.opacity is None else round(mat.opacity * 100)
        look = [("Opacity (%)", _edit(f"libmat:opacity:{name}", f"{opacity}")),
                ("Finish", str(mat.finish or "from the name")),
                ("Used by", f"{item.get('uses', 0)} object" + ("s" if item.get("uses", 0) != 1 else ""))]
        more = [(f"lib:duplicate:{key}", "Duplicate"), (f"lib:keep:{key}", "Add to My library"),
                (f"lib:delete:{key}", "Delete…")]
        return name, [("Material", rows), ("Actions", acts),
                      ("Surface · PBR (empty = off)", surface), ("Actions", maps),
                      ("Look", look), ("Actions", more)]

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
        elif field == "color":
            mat = ctx.scene.materials.get(name)
            if mat is not None and (mat.texture or {}).get("path"):
                ctx.core("edit_material", name=name, color=list(_parse_hex(text)))
            else:
                ctx.core("set_material_base", name=name, base="color", color=list(_parse_hex(text)))
        elif field == "tile":
            parts = [p for p in re.split(r"[^0-9.,]+", text.replace(",", ".")) if p]
            if not parts:
                raise ValueError("Type the tile size, like 1.2 × 0.6")
            w = float(parts[0])
            h = float(parts[1]) if len(parts) > 1 else w
            if w <= 0 or h <= 0:
                raise ValueError("A tile is larger than zero")
            ctx.core("edit_material", name=name, tile=[w, h])
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
    from PySide6.QtWidgets import QFileDialog
    path, _ = QFileDialog.getOpenFileName(win, title, "", "Images (*.png *.jpg *.jpeg *.webp *.bmp *.tif *.tiff)")
    return path or None


class ComponentsProvider(Provider):
    """Definitions in the model (copies counted, kept-in-file ones too),
    then the bundled starters and 2D people."""
    id, label = "components", "Components"
    apply_label = "Place a copy"
    empty = "No components yet."

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
            _select(ctx.win, groups)
            ctx.flash(f"{len(groups)} copies of {name} selected")
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
    empty = ("Profiles are coming: section outlines — steel shapes, mouldings, gaskets — "
             "to extrude, or run along a path with Follow Me.")


def _is_linked(g) -> bool:
    try:
        from core import links
        return links.is_linked(g)
    except Exception:   # noqa: BLE001 - a missing link module means "no"
        return False


def _materials_panel(win):
    return getattr(getattr(win, "tray", None), "materials", None)


def _rename_paint(old, new) -> None:
    from tools.paint import PaintTool
    mat = getattr(PaintTool, "current_material", None)
    if mat is not None and mat.name == old:
        from dataclasses import replace
        PaintTool.current_material = replace(mat, name=new)


#: The registry, in tab order.
PROVIDERS = [MaterialsProvider(), ComponentsProvider(), ProfilesProvider()]


def provider(type_id):
    return next((p for p in PROVIDERS if p.id == type_id), PROVIDERS[0])


# ---- state and rows ------------------------------------------------------------

def _state(view) -> dict:
    st = view.__dict__.get("_ui_lib")
    if st is None:
        saved = _settings_json("library/state", {})
        st = {"type": saved.get("type", "materials"), "view": int(saved.get("view", 0)),
              "sort": saved.get("sort", "name"), "filter": "", "fold": set(saved.get("fold", [])),
              "open": set(), "recent": _settings_json("library/recent", []), "mine_rev": 0, "cursor": None,
              "sig": None, "items": {}}
        if st["sort"] not in dict(SORTS):
            st["sort"] = "name"
        st["facet"] = saved.get("facet", "all")
        if st["type"] not in [p.id for p in PROVIDERS if p.ready]:
            st["type"] = "materials"      # "colours" was a type of its own before 2026-11-22
        view._ui_lib = st
    return st


def _save_state(st) -> None:
    _store_json("library/state", {"type": st["type"], "view": st["view"], "sort": st["sort"],
                                  "facet": st.get("facet", "all"),
                                  "fold": sorted(st["fold"])})


def _sorted(items, sort, recent):
    rank = {k: i for i, k in enumerate(recent)}
    if sort == "used":
        return sorted(items, key=lambda e: (-e["uses"], _natural(e.get("label") or e["name"])))
    if sort == "recent":
        return sorted(items, key=lambda e: (rank.get(e["key"], len(rank)), _natural(e.get("label") or e["name"])))
    return sorted(items, key=lambda e: _natural(e.get("label") or e["name"]))


def rows_for(items, st, current=None):
    """The display rows of one type's items: sections in ``SECTIONS`` order,
    categories inside, items sorted; folded headers hide their items. A
    filter shows every match unfolded. Returns ``(rows, item_by_key)``."""
    words = st["filter"].lower().split()
    # Browsing by base: the facet, or the tokens is:colour / is:image.
    bases = {"is:colour": "colour", "is:color": "colour", "is:image": "image"}
    base = next((bases[w] for w in words if w in bases), None)
    words = [w for w in words if w not in bases]
    facet = st.get("facet", "all")
    base = base or (facet if facet in ("colour", "image") else None)
    if base:
        items = [e for e in items if e.get("base", base) == base]
    if words:
        items = [e for e in items
                 if all(w in f"{e['name']} {e.get('label', '')} {e.get('short', '')} {e['category']}".lower()
                        for w in words)]
    by_key = {e["key"]: e for e in items}
    rows = []

    def row(kind, key, name, meta="", hexc="", icon="", verbs="", count=0, active=0, expanded=0):
        rows.append(([key, name, meta, hexc, icon, verbs], [kind, count, active, expanded]))

    for sec, title in SECTIONS:
        group = [e for e in items if e["source"] == sec]
        if not group and (words or sec in ("linked", "bundled")):
            continue
        sk = f"s:{sec}"
        open_ = bool(words) or sk not in st["fold"]
        row(0, sk, title, count=len(group), expanded=int(open_))
        if not open_:
            continue
        if not group:
            note = {"model": "Nothing of this type in the model yet.",
                    "mine": "Right-click an item of this model › Add to My library to keep it for every project."
                    }.get(sec, "")
            if note:
                row(3, f"n:{sec}", note)
            continue
        cats = {}
        for e in group:
            cats.setdefault(e["category"], []).append(e)
        names = sorted(cats, key=lambda c: (c != "", _natural(c)))
        for cat in names:
            members = _sorted(cats[cat], st["sort"], st["recent"])
            if cat:
                ck = f"c:{sec}:{cat}"
                # Bundled categories start folded; the model's and mine open.
                cat_open = bool(words) or (ck in st["open"] if sec == "bundled" else ck not in st["fold"])
                row(1, ck, cat, count=len(members), expanded=int(cat_open))
                if not cat_open:
                    continue
            for e in members[:ROW_CAP]:
                meta = " · ".join(x for x in (e["category"], dict(SECTIONS)[e["source"]],
                                              f"used by {e['uses']}" if e["uses"] else "") if x)
                # A tile has room for a short name (a RAL code); a list row for all of it.
                shown = (e.get("short") if st["view"] == 0 else None) or e.get("label") or e["name"]
                row(2, e["key"], shown, meta, e["hex"], e.get("icon", ""), ",".join(e["verbs"]),
                    count=e["uses"], active=int(current is not None and e["name"] == current))
            if len(members) > ROW_CAP:
                row(3, f"n:{sec}:{cat}:more", f"{len(members) - ROW_CAP} more — filter to find them")
    return rows, by_key


def _thumb(view, path) -> str:
    """A texture or thumbnail as a chrome icon, sharp on a Retina tile."""
    if not path:
        return ""
    key = "lib:" + str(path)
    sent = view.__dict__.setdefault("_ui_tex_icons", set())
    if key not in sent:
        sent.add(key)
        img = QImage(str(path))
        if not img.isNull():
            img = img.scaled(THUMB_PX, THUMB_PX, Qt.KeepAspectRatioByExpanding, Qt.SmoothTransformation)
            x, y = (img.width() - THUMB_PX) // 2, (img.height() - THUMB_PX) // 2
            img = img.copy(x, y, THUMB_PX, THUMB_PX).convertToFormat(QImage.Format_RGBA8888)
            view.renderer.ui_set_icons([(key, THUMB_PX, THUMB_PX, bytes(img.constBits())[: THUMB_PX * THUMB_PX * 4])])
    return key


def before_frame(view, win) -> None:
    """While the Library is the dock's place: send its rows when anything
    they show changed (type, view, sort, filter, folds, the model)."""
    if view.__dict__.get("_ui_mode") != "library" or not hasattr(view.renderer, "ui_set_library"):
        return
    from views.wgpu_ui import _current_material_name
    st = _state(view)
    scene = win.viewport.scene
    current = _current_material_name()
    sig = (st["type"], st["view"], st["sort"], st.get("facet"), st["filter"], tuple(sorted(st["fold"])), tuple(sorted(st["open"])),
           id(scene), scene.content_version, current, tuple(st["recent"][:12]), st["mine_rev"])
    if sig == st["sig"]:
        return
    st["sig"] = sig
    ctx = Ctx(win, view)
    counts, active_items = {}, []
    for p in PROVIDERS:
        if not p.ready:
            continue
        items = p.items(ctx)
        counts[p.id] = sum(1 for e in items if e["source"] in ("model", "linked"))
        if p.id == st["type"]:
            active_items = items
    prov = provider(st["type"])
    for e in active_items:
        e["icon"] = _thumb(view, e["thumb"])
    rows, by_key = rows_for(active_items, st, current if prov.id == "materials" else None)
    st["items"] = {e["key"]: e for e in active_items}
    view.renderer.ui_set_library(
        [(p.id, p.label, counts.get(p.id, 0), p.ready) for p in PROVIDERS], prov.id, st["view"],
        f"{st['sort']},facet:{st.get('facet', 'all')}" if getattr(prov, "facets", None) else st["sort"],
        SORTS + ([("-", "")] + prov.facets if getattr(prov, "facets", None) else []), prov.verbs(), prov.type_actions(ctx), prov.empty, st["filter"], rows)


def item(view, key):
    return _state(view)["items"].get(key)


# ---- opening and leaving ---------------------------------------------------------

def set_mode(view, win, mode) -> None:
    """The dock's place: ``"library"``, ``"problems"`` or ``""`` (the
    tree). Leaving the Library returns Project to the mode it had."""
    cur = view.__dict__.get("_ui_mode", "")
    if mode == "library":
        if cur != "library":
            view._ui_lib_back = cur
            _land(view, win)
        view.renderer.ui_library_opened() if hasattr(view.renderer, "ui_library_opened") else None
    elif mode == "" and cur == "library":
        mode = view.__dict__.get("_ui_lib_back", "") or ""
    view._ui_mode = mode
    view._ui_tree_sig = None
    view._ui_insp_sig = None
    view.update()


def _land(view, win) -> None:
    """Which type opens: Paint's material decides between Materials and
    Colours; a component being placed opens Components."""
    from tools.paint import PaintTool
    st = _state(view)
    tool = getattr(win.viewport, "active_tool", None) or getattr(getattr(win.viewport, "tools", None), "active", None)
    if type(tool).__name__ == "PlaceGroupTool":
        st["type"] = "components"
    elif _active_tool(win) == "paint":
        st["type"] = "materials"


def _active_tool(win) -> str:
    return next((k for k, a in getattr(win, "_tool_actions", {}).items() if a.isChecked()), "")


def open_library(win, type_id=None) -> None:
    """Window › Library › <type>, ⌘K and the rail: the dock shows the
    Library on that type, its filter ready for typing."""
    view = getattr(getattr(win, "_doc", None), "stand_in", None)
    if view is None or not getattr(view, "_ui_attached", False):
        return
    if type_id:
        _state(view)["type"] = type_id
    set_mode(view, win, "library")
    if type_id:
        _state(view)["type"] = type_id
        _state(view)["sig"] = None


def install(win) -> None:
    """Window › Library › Materials / Colours / Components: menu entries
    (so ⌘K finds them) and the rail's Components button."""
    from PySide6.QtGui import QAction
    if getattr(win, "_lib_installed", False):
        return
    win._lib_installed = True
    menu = None
    for a in win.menuBar().actions():
        if a.menu() is not None and a.text().replace("&", "") in ("Window", "Ventana"):
            menu = a.menu()
            break
    acts = {}
    for p in PROVIDERS:
        if not p.ready:
            continue
        act = QAction(p.label, win)
        act.triggered.connect(lambda _=False, t=p.id: open_library(win, t))
        acts[p.id] = act
    if menu is not None:
        sub = menu.addMenu("Library")
        for act in acts.values():
            sub.addAction(act)
    comp = QAction("Components", win)
    comp.setToolTip("Components: the Library's components, to place")
    comp.triggered.connect(lambda: open_library(win, "components"))
    win._act_library_components = comp
    win._ui_commands = None
    _register_icon()


def _register_icon() -> None:
    """The rail's Components glyph: a box, drawn like the other tool icons."""
    from views import icons
    if "components" in icons._DRAW:
        return
    from PySide6.QtCore import QPointF

    def draw(p, _ink):
        top = [QPointF(24, 8), QPointF(39, 16), QPointF(24, 24), QPointF(9, 16)]
        p.drawPolygon(top)
        p.drawLine(QPointF(9, 16), QPointF(9, 32))
        p.drawLine(QPointF(39, 16), QPointF(39, 32))
        p.drawLine(QPointF(24, 24), QPointF(24, 40))
        p.drawLine(QPointF(9, 32), QPointF(24, 40))
        p.drawLine(QPointF(39, 32), QPointF(24, 40))
    icons._DRAW["components"] = draw


# ---- actions ------------------------------------------------------------------

def _ask(view, pending, title, body, yes) -> None:
    view._ui_pending = ("lib", pending)
    view.renderer.ui_set_prompt(title, body, [("yes", yes), ("no", "Cancel")])


def confirmed(win, view, pending, text="") -> None:
    """The dock prompt answered yes for a Library question."""
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
    """``verb SEP key [SEP arg]`` from the browser (``verb:key`` from an
    inspector chip)."""
    if view is None:
        return
    if SEP not in arg and ":" in arg:
        verb, _, rest = arg.partition(":")
        arg = verb + SEP + rest
    verb, _, rest = arg.partition(SEP)
    key, _, extra = rest.partition(SEP)
    st = _state(view)
    ctx = Ctx(win, view)
    try:
        if verb == "type":
            st["type"] = key if any(p.id == key and p.ready for p in PROVIDERS) else st["type"]
            st["filter"] = ""
            view._ui_mat_focus = None
            _save_state(st)
        elif verb == "view":
            st["view"] = 1 if key == "list" else 0
            _save_state(st)
        elif verb == "sort" and key.startswith("facet:"):
            st["facet"] = key[6:] if key[6:] in ("all", "colour", "image") else "all"
            _save_state(st)
        elif verb == "sort":
            st["sort"] = key if key in dict(SORTS) else "name"
            _save_state(st)
        elif verb == "filter":
            st["filter"] = key
        elif verb == "toggle":
            sec = key.split(":")[1] if key.count(":") >= 1 else ""
            if key.startswith("c:") and sec == "bundled":
                st["open"].symmetric_difference_update({key})
            else:
                st["fold"].symmetric_difference_update({key})
            _save_state(st)
        elif verb == "typeact":
            provider(st["type"]).type_action(ctx, key)
        elif verb == "focus":
            st["cursor"] = key
            it = item(view, key)
            view._ui_mat_focus = key if it is not None else None
        else:
            it = item(view, key)
            if it is None:
                return
            p = provider(st["type"])
            if verb == "apply":
                _remember(view, key)
                st["cursor"] = key
                view._ui_mat_focus = key
            p.run(ctx, verb, it, extra)
        st["sig"] = None
        view._ui_insp_sig = None
    except (KeyError, ValueError) as exc:
        ctx.flash(str(exc).strip("'\""))
    view.update()


# ---- inspector -------------------------------------------------------------------

def inspector(view, win):
    """``(title, sections)`` for the Library item in focus, or None (the
    inspector then shows the selection as usual)."""
    key = view.__dict__.get("_ui_mat_focus")
    it = item(view, key) if key else None
    if it is None:
        return None
    return provider(_state(view)["type"]).inspector(Ctx(win, view), it)


def edit_field(win, view, arg, text) -> None:
    """An inspector field of a Library item: ``<field>:<material name>``."""
    field, _, name = arg.partition(":")
    st = _state(view)
    it = st["items"].get(f"m:{name}") or {"key": f"m:{name}", "name": name}
    p = provider(st["type"])
    p.edit(Ctx(win, view), it, field, text)
    st["sig"] = None
