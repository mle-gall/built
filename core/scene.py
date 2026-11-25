# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Scene container, backed by a shared-vertex :class:`~core.mesh.Mesh`.

``edges`` and ``faces`` are read-only views onto the mesh (lists of
``mesh.Edge`` / ``mesh.Face``), so render, bounds and ``.igz`` save consume them
unchanged. Every mutation goes through mesh methods (via the ``Command`` layer),
which keep shared-vertex connectivity and incidence in sync — no more
position-matching to rediscover topology.

``version`` bumps on every mutation so the viewport can cheaply decide whether
to rebuild its dynamic VBOs.
"""
from __future__ import annotations
import ingetrazo_core
from ingetrazo_core.mesh import element_identity
from ingetrazo_core import ElementSet

from dataclasses import InitVar, dataclass, field
from typing import Iterable

import ingetrazo_core as _NATIVE
from ingetrazo_core import Vec3 as QVector3D

from core import layers as _layers
from core import doc_model as _dm
from ingetrazo_core.mesh import Edge, Face, Mesh


def _make_style():
    from core.style import Style
    return Style()


def _make_shadows():
    from core.sun import ShadowSettings
    return ShadowSettings()


def _make_animation():
    from ingetrazo_core import SceneAnimation
    return SceneAnimation()


@dataclass
class Scene:
    def __setattr__(self, name, value):
        # All command/tool assignments share the same key-based membership.
        if name == "selection" and not isinstance(value, ElementSet):
            value = ElementSet(value)
        object.__setattr__(self, name, value)

    mesh: Mesh = field(default_factory=Mesh)
    selection: ElementSet = field(default_factory=ElementSet)
    version: int = 0
    #: How many of ``version``'s bumps changed only what is SHOWN -- the
    #: selection -- and not the document. The GL caches key on ``version``
    #: and need every bump; "unsaved changes" must not: a click on empty
    #: space after Ctrl+S asked to save again (issue #159).
    view_version: int = 0
    _scene_catalog_ticks: int = field(default=0, init=False, repr=False)
    # Context changes which preserve resident placement geometry.
    _placement_context_bumps: int = field(default=0, init=False, repr=False)
    #: How many of ``version``'s bumps moved only the section planes —
    #: placed, moved, turned, switched. They ARE document edits (the planes
    #: are saved), but no face, edge or object changed: the caches of what
    #: the model is (``geometry_version``) stay. Every drag of a section's
    #: arrow rebuilt the pick index of the whole model, ~2 s a move on a
    #: furnished house.
    section_version: int = 0
    # Encapsulated chunks (own meshes), isolated from the main mesh's welding.
    # With the Rust document (core/doc_model.py) ``groups`` and ``layers``
    # are the lists of the scene's root node, made with the scene; the
    # constructor still takes them.
    _root: object = field(default_factory=_dm.new_scene_root,
                          repr=False, compare=False)
    groups: InitVar[list | None] = None
    # Annotation entities (static dimensions) — not geometry, drawn as overlays.
    dimensions: list = field(default_factory=list)
    # Leader-text annotations (the Text tool) — same overlay treatment.
    text_labels: list = field(default_factory=list)
    text_placement_anchors: list = field(default_factory=list)
    # Georef traced paths (roads / boundaries / alignments) — first-class georef
    # entities, kept out of the topology mesh entirely (Track G).
    geo_paths: list = field(default_factory=list)
    # Imported survey points (GPS / total station, UTM CSV) — reference markers
    # the trace snaps to; never part of the mesh (Track G, municipal flow).
    geo_points: list = field(default_factory=list)
    # Construction guides (Tape Measure): infinite dashed lines / points used to
    # align real drawing. Scaffolding, never part of the mesh.
    guides: list = field(default_factory=list)
    # Imported reference images (core.image_plane.ImagePlane) — a scanned plan
    # or photo you trace over. Display-only like the terrain and for the same
    # reason (invariant #4): reference to draw on top of, never topology.
    image_planes: list = field(default_factory=list)
    # Layers / tags: labels with visibility + lock. The default
    # layer always exists; entities reference layers by KEY (Layer.key —
    # the name they were made with), the name being only a label.
    layers: InitVar[list | None] = None
    # The layer new objects go to (core.layers.active_layer); None = the
    # default layer.
    active_layer: str | None = None
    # Folder names of the Layers panel, in order — empty folders included
    # (a layer names its own folder in ``Layer.folder``).
    layer_folders: list = field(default_factory=list)
    # Folder → the folder it sits in (absent: at the top).
    folder_parents: dict = field(default_factory=dict)
    folder_visibility: dict = field(default_factory=dict)
    layer_folder_uids: dict = field(default_factory=dict)
    # Folder key → its label, for the folders whose label is not their key
    # (core.layers.folder_label).
    layer_folder_names: dict = field(default_factory=dict)
    # Hatch patterns brought in from pattern files (core.hatches.parse_pat),
    # name → {"label", "families"}: a layer's ``hatch`` names one as
    # ``pat:<name>``, and the document carries it along.
    hatch_patterns: dict = field(default_factory=dict)
    # Line types the user made or brought in from line type files
    # (core.layers.parse_lin), name → {"label", "pattern"} in paper mm: a
    # layer's ``linetype`` names one as ``lin:<name>``.
    line_types: dict = field(default_factory=dict)
    # The colours given to the built-in line types in this document
    # (core.layers.LINETYPES key → (r, g, b)): a dashed line drawn red,
    # whatever its layer. An own line type keeps its colour in its entry.
    linetype_colors: dict = field(default_factory=dict)
    # The document's masonry pieces, kind → (length, width, height, joint)
    # in mm («brick», «hollow», «block»; core.hatches.piece_kind): what every
    # layer's masonry hatch is drawn with unless the layer sets its own
    # (core.layers.layer_brick). A kind not here: its usual piece.
    bricks: dict = field(default_factory=dict)
    # Named materials (core.materials.Material), name → Material. The
    # registry gives identity to paint recipes; faces keep their baked
    # attrs (color/texture) as the render truth and optionally carry
    # attrs["mat"] = name. See core/materials.py.
    materials: dict = field(default_factory=dict)
    # Saved views ("Scenes"): named camera + layer-visibility
    # snapshots (core.saved_views.SavedView). Presentation state, no geometry.
    saved_views: list = field(default_factory=list)
    # Folder names of the Scenes tab, in order — empty folders included (a
    # scene names its own folder in ``SavedView.folder``), and folder → the
    # folder it sits in (absent: at the top). See ingetrazo_core.scene_tree.
    scene_folders: list = field(default_factory=list)
    scene_folder_parents: dict = field(default_factory=dict)
    # Sheet compositions (core.composition.Composicion) — the print layouts.
    compositions: list = field(default_factory=list)
    # Scales (1:N) typed by the user in the composer beyond the common
    # presets — kept with the document so every frame of the file offers
    # them again (Marco, 2026-09-02).
    custom_scales: list = field(default_factory=list)
    #: Component definitions the document keeps with no copy placed —
    #: each a placement-free template :class:`~core.group.Group` (identity
    #: ``xform``) that shares its prototype with any copies still in the
    #: model. They never draw, pick or export; they wait in the «In model»
    #: list to be placed again or to replace other copies. See
    #: :mod:`core.kept_components`.
    kept_components: list = field(default_factory=list)
    # Display style for dimension annotations (edited from the Tray).
    #: ``norma`` is the drafting standard the dimensions obey — ISO (and so
    #: the Spanish UNE) or the German/Japanese one. It belongs to the
    #: DOCUMENT and not to the machine: a setting kept in QSettings would
    #: redraw a colleague's dimensions the moment he opened the file
    #: (Marco, 2026-09-17). Absent from an older .igz = ISO, which is what
    #: those files were drawn with.
    #: ``base_step_mm`` is how far apart the rows of a baseline run sit on
    #: PAPER — AutoCAD's DIMDLI, and Marco asked for it as a number he
    #: sets rather than one derived from the text height (2026-09-19).
    #: ``ends`` is what closes the dimension line — ``arrow`` (Rafael's
    #: sheet, the default), ``tick`` (oblique, what every document drew
    #: before the choice existed: a file without the key opens with it)
    #: or ``none``. Asked for by a user of DriveMeca's video («los
    #: extremos no tiene para cambiarla», 2026-09-20).
    #: The camera the document was last saved with — target, distance, yaw,
    #: pitch, fov, perspective — so opening it shows what its author saw
    #: (.skp files keep the camera too). ``None`` = never saved.
    #: @pacaeiro, issue #60: «If I do a New drawing, or open a drawing, the
    #: Camera stays in the position where it was before».
    camera_home: dict | None = None
    #: How many views it opens in and where the lines between them are
    #: (ingetrazo_render.DocumentViews); None: one view. Travels in the .igz like the
    #: camera, and like the camera changing it is not an edit.
    views_home: dict | None = None
    #: The model's units (issue #33): the unit a bare number is typed in and
    #: every length is shown in, plus the decimals. Travels in the .igz;
    #: read through ``core.units`` (``fmt_len`` & co.).
    units: dict = field(default_factory=lambda: {"length": "m", "precision": 2})
    #: Extensions' own document data, one JSON-safe value per extension key
    #: (``views.extension_api.ExtensionApp.document_data``). Travels in the
    #: .igz; the core never reads it. Changed through
    #: ``core.history.SetPluginDataCommand`` so each edit is undoable.
    plugin_data: dict = field(default_factory=dict)
    #: The linked files' registry (core/links.py): per source path, its tag
    #: overrides, the tags and folders it brought, whether it is packed.
    #: Saved as ``plugin_data["links"]``.
    link_registry: dict = field(default_factory=dict)
    dimension_style: dict = field(default_factory=lambda: {
        "decimals": 2, "units": "m", "font_size": 9, "color": [45, 55, 75],
        "norma": "iso", "base_step_mm": 8.0, "ends": "arrow",
        "show_unit": True})
    # Back-face tint override (RGB 0..1), e.g. adopted from an imported
    # .skp's style so unpainted faces read like they did for the author.
    # ``None`` = the viewport's default blue-grey.
    back_face_color: tuple | None = None
    # Active display style (Styles): face mode, edges, background.
    # The viewport reads it every frame; scenes snapshot it (core/style.py).
    display_style: object = field(default_factory=lambda: _make_style())
    # Sun shadows (core/sun.py): whether they draw, and the local date/time
    # the sun stands at. Document data — the shadow study is a deliverable.
    shadows: object = field(default_factory=lambda: _make_shadows())
    # Scene transitions: whether showing a scene
    # glides there, and how long the glide and a run's pause on each take.
    animation: object = field(default_factory=lambda: _make_animation())
    # Section planes (core/section.py). At most ONE is
    # ``active`` (the cut) in the model context; the two flags mirror
    # View ▸ Section Planes / Section Cuts toggles.
    section_planes: list = field(default_factory=list)
    show_section_planes: bool = True
    # View ▸ Hidden Objects / Hidden Geometry: hidden objects
    # (groups, components) and hidden geometry (faces, edges) are drawn
    # as a see-through grid and become selectable — the way back to
    # Unhide ▸ Selected. Never drawn normally: ``entity_visible`` stays
    # False for hidden things; the ghost pass draws them.
    show_hidden_objects: bool = False
    show_hidden_geometry: bool = False
    # Camera ▸ Axes: the world's red/green/blue drawing axes. Only the
    # drawing hides — axis inference and the arrow-key locks still work,
    # and an open group keeps showing its own axes.
    show_axes: bool = True
    # Tools ▸ North without a location: where true north lies, degrees
    # clockwise from the green (Y) axis — the sun turns with it. Once the
    # model has a location the datum's own north angle is the one that
    # counts (core.sun.scene_north_deg).
    north_deg: float = 0.0
    # Camera ▸ Component Edit ▸ Hide Similar Components: while a
    # component instance is open, its sibling instances (the other copies of
    # the same definition) leave the frame instead of fading with the rest.
    # A view preference, not document content: only read while editing.
    hide_similar_components: bool = False
    show_section_cuts: bool = True
    # Georeferencing anchor (Track G). ``None`` until the user sets a datum;
    # once set, geodetic ↔ local-metre conversion goes through it. Terrain and
    # tiles are separate display-only objects added in later phases.
    georef: object | None = None
    # Base-map tile layer (Track G, G1) — display-only, never welded into the
    # mesh. Runtime state (not serialised as geometry); requires ``georef``.
    tile_layer: object | None = None
    # 3D draped terrain (Track G, G2 full) — display-only relief mesh, runtime.
    terrain: object | None = None
    # Photogrammetric survey (Track G, G6) — the drone flight's own textured
    # mesh from WebODM/ODM. Display-only like the terrain, and for the same
    # reason: hundreds of thousands of reconstruction triangles are reference
    # geometry to trace over, never topology-engine geometry (invariant #4).
    photo_mesh: object | None = None
    # BIM "active class" (tag-as-you-draw): while set, faces created by the
    # drawing tools are stamped with this tag ({"id", "class", "name"}) at
    # commit time. Runtime UI mode — never serialised.
    active_ifc: object | None = None
    # Group-edit context (Groups v2): while set, ``mesh`` POINTS AT the edited
    # group's mesh so every tool/command works inside the group transparently;
    # ``_loose_mesh`` keeps the real loose mesh for render and restore.
    # ``edit_group`` is the INNERMOST open group — it mirrors the top of
    # ``_edit_stack``, so the thirty-odd readers that only care about "am I
    # inside a group" keep working unchanged.
    edit_group: object | None = None
    _loose_mesh: object | None = None
    #: The open contexts, outermost first — nested editing. Each
    #: entry pins the placement, its parent edit mesh and its drawing frame.
    @property
    def _edit_stack(self):
        return self._root._edit_levels

    def __post_init__(self, groups, layers):
        if groups is not None:
            self.groups = groups
        if layers is not None:
            self.layers = layers

    def __deepcopy__(self, memo):
        import copy
        out = object.__new__(type(self))
        memo[id(self)] = out
        for name, value in self.__dict__.items():
            if name != "_root":
                object.__setattr__(out, name, copy.deepcopy(value, memo))
        object.__setattr__(out, "_root", _dm.NATIVE.doc_new_root())
        for level in self._edit_stack:
            copied = copy.deepcopy(level, memo)
            out._root._enter_edit(copied["group"], copied)
        out.groups = copy.deepcopy(list(self.groups), memo)
        out.layers = copy.deepcopy(list(self.layers), memo)
        for name in ROOT_TABLES:
            setattr(out, name, copy.deepcopy(getattr(self, name), memo))
        out.materials = copy.deepcopy(dict(self.materials), memo)
        out.kept_components = copy.deepcopy(list(self.kept_components), memo)
        return out

    # ---- Geometry views (read-only over the *loose* mesh) -------------------
    # Tools, edits and topology operate on this (the loose geometry); groups are
    # walled off so drawing never welds to them.
    @property
    def edges(self) -> list[Edge]:
        return self.mesh.edges

    @property
    def faces(self) -> list[Face]:
        return self.mesh.faces

    # ---- Layers --------------------------------------------------------------
    def layer_by_key(self, key: str):
        """The tag whose key is ``key`` — what entities carry."""
        if isinstance(key, str):
            return self._root._tag_by_key(key)
        for ly in self.layers:
            if ly.key == key:
                return ly
        return None

    def layer(self, name: str):
        """The tag ``name`` names: by its key (what entities carry), else
        by its label — the first with it, labels being free to repeat."""
        if not name:
            return None
        ly = self.layer_by_key(name)
        if ly is not None:
            return ly
        if isinstance(name, str):
            return self._root._tag_by_name(name)
        for ly in self.layers:
            if ly.name == name:
                return ly
        return None

    def _layer_state(self, entity) -> tuple[bool, bool]:
        """(visible, locked) of the layer ``entity`` carries; unknown layer
        names read as the default (visible, unlocked)."""
        name = _layers.layer_of(entity)
        if name == _layers.DEFAULT_LAYER and self.edit_group is not None:
            name = self._open_group_layer(entity) or name
        ly = self.layer_by_key(name)
        if ly is None:
            return True, False
        return ingetrazo_core.tag_edits.effective_visible(self, ly), ly.locked

    def _open_group_layer(self, entity):
        """The layer a face or edge with none of its own follows while its
        group is open: the open group's, or the nearest enclosing group's
        that has one. Closed, the group's layer already decides for
        everything in it; open, its geometry is drawn and picked as loose
        geometry, and hiding the default layer took the contents of a
        group on another layer with it (#366)."""
        if hasattr(entity, "children") or not (
                hasattr(entity, "attrs") or hasattr(entity, "v0")):
            return None
        group_layer = None
        from core.outliner import open_path
        for group in reversed(open_path(self)):
            group_layer = getattr(group, "layer", None)
            if group_layer:
                break
        if not group_layer:
            return None
        key = (id(self.mesh), self.version, len(self.mesh.faces),
               len(self.mesh.edges))
        memo = getattr(self, "_open_mesh_ids", None)
        if memo is None or memo[0] != key:
            ids = {element_identity(f) for f in self.mesh.faces}
            ids.update(element_identity(e) for e in self.mesh.edges)
            memo = self._open_mesh_ids = (key, ids)
        return group_layer if element_identity(entity) in memo[1] else None

    def open_group_paint(self):
        """The paint the open group's default faces wear: its own, or the
        nearest enclosing open group's that has one — what they wore with
        the group closed (#1101), as :meth:`_open_group_layer` does for
        the layer. ``None`` outside a group or when nothing is painted."""
        from core.outliner import open_path
        for group in reversed(open_path(self)):
            paint = getattr(group, "material", None)
            if paint:
                return paint
        return None

    @staticmethod
    def _object_hidden(entity) -> bool:
        """Hide on an OBJECT (a group or component). Edges carry
        a ``hidden`` of their own with older, narrower semantics (they stay
        in the topology and the draw passes skip them themselves), so only
        a group answers here."""
        return bool(getattr(entity, "hidden", False)) \
            and hasattr(entity, "children")

    @staticmethod
    def _face_hidden(entity) -> bool:
        """Hide on a face: ``attrs["hidden"]``."""
        attrs = getattr(entity, "attrs", None)
        return bool(attrs and attrs.get("hidden"))

    def entity_hidden(self, entity) -> bool:
        """Hidden by Hide (object or face) — regardless of the layer, and of
        whether the hidden-things view is on."""
        return self._object_hidden(entity) or self._face_hidden(entity)

    def entity_visible(self, entity) -> bool:
        from core.group import Group
        allowed = getattr(self, '_navigator_allowed', None)
        if allowed is not None and isinstance(entity, Group) and self.navigator_key(entity) not in allowed:
            return False
        # A hidden thing is gone from every consumer that asks this —
        # render, pick, snap, bounds, export — whatever its layer says.
        # The hidden-things view does NOT change that: it draws them as a
        # ghost in its own pass and only makes them selectable.
        if self.entity_hidden(entity) or self.similar_hidden(entity):
            return False
        return self._layer_state(entity)[0]

    def edge_snappable(self, edge) -> bool:
        """Whether the inference may land on ``edge`` — the visibility rule
        the render, ``bounds()`` and the pick already follow.

        An EDGE can be hidden on its own (Hide with the eraser, the seams of
        a smoothed surface) without hiding what it bounds, and the draw
        passes skip it: no line, no profile. The snap engine did not ask, so
        the point offered to the cursor existed nowhere on screen — and a
        line drawn to it landed on a spot the user could not see, cutting
        the face it lay on (the "phantom vertex" segmentations). Hidden
        EDGES come back for inference only with Camera ▸ Hidden Geometry
        on, the same switch that draws them as a ghost and makes them
        selectable.

        A visible, plain edge is unaffected: nothing here filters curves
        (``soft``), locked geometry, guides or group edges, which have their
        own rules.
        """
        if not self.entity_visible(edge):
            return False
        return (not getattr(edge, "hidden", False)
                or bool(getattr(self, "show_hidden_geometry", False)))

    def group_snappable(self, group) -> bool:
        """Visible group geometry is a read-only drawing reference, even
        when the group or its tag is locked. Revealed editable geometry
        keeps its existing inference behavior."""
        return self.entity_visible(group) or self.entity_selectable(group)

    def edit_definition(self):
        """The shared prototype mesh of the component instance open at the
        innermost level, or ``None`` (at the root, or inside a classic
        group)."""
        if not self._edit_stack:
            return None
        return self._edit_stack[-1].get("definition")

    def similar_hidden(self, entity) -> bool:
        """Out of the frame by Hide Similar Components: another instance of
        the definition being edited (or a nested placement of one). Nothing
        is hidden at the root, so saving and exporting never see it."""
        if not self.hide_similar_components or not self._edit_stack:
            return False
        if not hasattr(entity, "children") or entity is self.edit_group:
            return False
        definition = self.edit_definition()
        if definition is None:
            return False
        for g in (entity, getattr(entity, "owner", None)):
            if (g is not None and g is not self.edit_group
                    and getattr(g, "xform", None) is not None
                    and getattr(g, "mesh", None) is definition):
                return True
        return False

    def entity_selectable(self, entity) -> bool:
        from core.group import Group
        allowed = getattr(self, '_navigator_allowed', None)
        if allowed is not None and isinstance(entity, Group) and self.navigator_key(entity) not in allowed:
            return False
        if self.similar_hidden(entity):
            return False
        if self._object_hidden(entity) and not self.show_hidden_objects:
            return False
        if self._face_hidden(entity) and not self.show_hidden_geometry:
            return False
        if self.entity_locked(entity) or (isinstance(entity, (Edge, Face)) and self._root._loose_locked()):
            return False
        visible, locked = self._layer_state(entity)
        return visible and not locked

    @staticmethod
    def entity_locked(entity) -> bool:
        """Locked as an object: the group itself, or — for a placement the
        viewport picks as part of its parent — the object that owns it."""
        for g in (entity, getattr(entity, "owner", None)):
            if g is not None and getattr(g, "locked", False) \
                    and hasattr(g, "children"):
                return True
        return False

    def navigator_key(self, group):
        return getattr(self, '_navigator_keys', {}).get(group, group.uid)

    def repeated_uids(self):
        return self._root._repeated_uids()

    def occurrence_closure(self, wanted, exact=False):
        """Isolating occurrence keys, from the native tree (doc/occurrences.rs):
        (matched keys, shown keys, drawn-through keys, {group: address key})."""
        matched, allowed, through, qualified = self._root._occurrence_closure(set(wanted), exact)
        return matched, allowed, through, dict(qualified)

    def occurrence_path(self, group):
        """Return the UIDs from this scene's root through the placement."""
        return self._root._occurrence_path(group)

    def resolve_occurrence(self, path):
        """Look up an ordered path of persisted placement UIDs from this root."""
        return self._root._resolve_occurrence(path)

    def groups_by_uid(self) -> dict:
        """``uid → group`` over the whole tree (nested placements too)."""
        from ingetrazo_core import purge_iter_groups as iter_groups
        return {g.uid: g for g in iter_groups(self.groups)}

    # ---- Sections (section planes) -------------------------------------------
    def active_section(self):
        """The section plane currently cutting the model, or ``None``."""
        for sp in self.section_planes:
            if sp.active:
                # A plane on a hidden layer cuts nothing (#316).
                return sp if self._layer_state(sp)[0] else None
        return None

    def set_active_section(self, plane) -> None:
        """Make ``plane`` the ONE active cut (None deactivates all) —
        one active cut per context."""
        for sp in self.section_planes:
            sp.active = sp is plane

    # ---- Group-edit context (Groups v2) --------------------------------------
    def begin_group_edit(self, group) -> None:
        """Enter a group: tools and commands now edit ITS mesh (the usual
        double-click-into-group).

        Entering a CHILD of the group already open pushes a level instead of
        starting over — that is the nesting. Anything else closes what is
        open first.

        A group that owns children used to be BAKED on the way in
        (``materialize``): its nine nested groups became one mesh, and both
        the structure and the per-group chunks were gone. «Es lo que no
        quiero, que se fundan todos los grupos, porque además pierdo
        rendimiento» (Marco, 2026-09-11). Now it enters as it is.

        The active store presents the definition in the occurrence frame.
        Other placements keep reading the same definition while tools edit it."""
        anidando = (self.edit_group is not None
                    and group in (getattr(self.edit_group, "children", None) or ()))
        if self.edit_group is not None and not anidando:
            self.end_group_edit()
        if not anidando:
            # Tree and protocol actions can open a nested row directly.
            # Enter its ancestors just as a sequence of double-clicks would.
            from core.group import Group
            parent = group._parent
            if isinstance(parent, Group):
                self.begin_group_edit(parent)
        self._refresh_dimension_holds()
        attachments = ()
        entry_mesh = group.mesh
        anterior = self.mesh
        # The context's own axes (issue #44), read BEFORE anything below
        # rewrites the placement: drawing inside a group happens on the
        # group's axes, level by level.
        from core.group import group_frame
        frame = group_frame(group)
        # The definition its sibling instances share, read before the entry
        # below swaps the instance onto a world copy (Hide Similar).
        is_comp = getattr(group, "is_component", None)
        definition = (group.mesh if callable(is_comp) and is_comp()
                      else None)
        parts = None
        active_mesh = None
        if (group.xform is not None or (
                self._edit_stack and self._edit_stack[-1].get("affine"))):
            from ingetrazo_core.store_mesh import instance_view
            world = self._root._edit_matrix(group)
            active_mesh = instance_view(group.mesh, world)
            if active_mesh is not None:
                frame = active_mesh._store.tool_placement()
                if group.axes is not None:
                    frame = frame * group.axes
        share = entry_transaction = None
        if active_mesh is None and (group.children or group.xform is not None):
            attachments = self._attachment_swap()
            from ingetrazo_core.command_protocol import enter_reference
            from ingetrazo_core.store_mesh import StoreMesh
            # A singular placement needs the compatibility world copy. Keep
            # the placement swaps on entry so its History step can replay them.
            depth = _dm.begin() if type(entry_mesh) is StoreMesh else 0
            try:
                share, parts = enter_reference(self, group, frame)
                if depth:
                    entry_transaction = _dm.commit()
            except Exception:
                if depth:
                    _dm.rollback()
                raise
        if not self._edit_stack:
            self._loose_mesh = anterior
        level = {"group": group, "mesh": anterior, "affine": active_mesh is not None, "session": object(),
                 "share": share, "entry_transaction": entry_transaction,
                 "frame": frame, "definition": definition, "parts": parts,
                 "world_copy": group.mesh if entry_mesh is not group.mesh else None}
        self._root._enter_edit(group, level)
        self._placement_context_bumps += 1
        self.mesh = active_mesh if active_mesh is not None else group.mesh
        self.edit_group = group
        self._restore_attachments(attachments)
        self._refresh_dimension_holds()
        self.selection.clear()
        self.version += 1

    def _attachment_swap(self):
        refs = [(label, "attachment") for label in self.text_labels]
        from core.composition import sheet_holds
        for sheet in self.compositions:
            for item in (*sheet.cotas, *sheet.etiquetas, *sheet.niveles):
                for name, (_point, ref) in sheet_holds(item):
                    if not isinstance(ref, dict):
                        refs.append((item, "hold_" + name))
        refs.extend((sign, "attachment") for sign in self.text_placement_anchors)
        refs.extend((dim, name) for dim in self.dimensions
                    for name in ("attachment_a", "attachment_b"))
        result = []
        for obj, name in refs:
            value = getattr(obj, name, None)
            if value is not None:
                ref = value[1] if name.startswith("hold_") else value
                result.append((obj, name, ref, ref.before_mesh_swap()))
        return result

    @staticmethod
    def _restore_attachments(refs):
        for obj, name, ref, saved in refs:
            replacement = ref.after_mesh_swap(saved)
            if replacement is not ref:
                if name.startswith("hold_"):
                    replacement = (getattr(obj, name)[0], replacement)
                setattr(obj, name, replacement)

    def _refresh_dimension_holds(self) -> None:
        """Read every dimension's endpoints once, either side of an edit
        context swapping a group's mesh (#621). Before the swap, the last
        known spots are made current; after it, a hold on the mesh that
        went away finds the vertex now standing there — before any edit
        can move it out from under that spot."""
        for d in self.dimensions:
            d.a, d.b

    def end_one_group_edit(self) -> None:
        """Leave the INNERMOST group only — Esc, which steps out
        one level and leaves you inside the parent."""
        self._leave_level()

    def end_group_edit(self) -> None:
        """Leave every open group-edit context, back to the loose mesh. The
        contract the whole app relies on before saving, exporting or
        switching documents: when this returns, nothing is open."""
        while self._edit_stack:
            self._leave_level()

    def _leave_level(self) -> None:
        if not self._edit_stack:
            return
        self._refresh_dimension_holds()
        attachments = self._attachment_swap() if self._edit_stack[-1].get("share") else ()
        nivel = self._root._leave_edit()
        unchanged = True
        share = nivel.pop("share", None)
        if share is not None:
            from ingetrazo_core.command_protocol import finish_reference
            unchanged = finish_reference(share, nivel.get("parts"))
        if unchanged:
            self._placement_context_bumps += 1
        self.mesh = nivel["mesh"]
        self.edit_group = (self._edit_stack[-1]["group"] if self._edit_stack
                           else None)
        if not self._edit_stack:
            self._loose_mesh = None
        self._restore_attachments(attachments)
        self._refresh_dimension_holds()
        self.selection.clear()
        self.version += 1

    @property
    def drawing_frame(self):
        """The axes drawing happens on (issue #44): the world's (``None``)
        at the top level, the open group's own axes inside it — a world
        matrix whose columns are red, green, blue and whose translation is
        the origin. Read by :mod:`core.axes`."""
        if self._edit_stack:
            return self._edit_stack[-1].get("frame")
        return None

    @property
    def loose_mesh(self):
        """The real loose mesh regardless of the edit context."""
        return self._loose_mesh if self.edit_group is not None else self.mesh

    # ---- Render views (loose + every group) ---------------------------------
    def placements(self, *, precise=False):
        """Every visible placement in the scene: each group and, below it,
        the nested ones a component keeps inside itself, as
        ``(group, world_matrix_or_None)``.

        Face-me billboards are left out (they are drawn per frame, not from
        their mesh). Consumers that used to walk ``self.groups`` and read
        ``g.mesh`` want this — otherwise the geometry a component places
        inside itself is simply invisible to them.

        A nested placement that is not visible takes its subtree along, as
        a hidden top-level group always did: a group on a hidden tag two
        levels down left its own children on screen."""
        def walk(pg, base):
            m = getattr(pg, "xform", None)
            if base is not None:
                m = base if m is None else base * m
            yield pg, m
            for child in getattr(pg, "children", None) or ():
                if getattr(child, "billboard", False) \
                        or not self.entity_visible(child):
                    continue
                yield from walk(child, m)

        for g in self.groups:
            if not self.entity_visible(g) or getattr(g, "billboard", False):
                continue
            if precise:
                yield from ingetrazo_core.group_model_placements(g, visible=lambda child:
                                            not child.billboard and self.entity_visible(child))
            else:
                yield from walk(g, None)

    def render_edges(self):
        for e in self.loose_mesh.edges:
            if self.entity_visible(e):
                yield e
        for g, _m in self.placements():
            yield from g.mesh.edges

    def render_faces(self):
        for f in self.loose_mesh.faces:
            if self.entity_visible(f):
                yield f
        for g, _m in self.placements():
            yield from g.mesh.faces

    # ---- Mutations ----------------------------------------------------------
    def add_edge(self, a: QVector3D, b: QVector3D) -> Edge:
        edge = self.mesh.add_edge(a, b)
        self.version += 1
        return edge

    def select(self, edges: Iterable, additive: bool = False,
               mode: str | None = None) -> None:
        """Put *edges* (any entities) into the selection the way *mode*
        says — the usual click modifiers: ``"replace"`` (a plain click),
        ``"add"`` (Ctrl), ``"toggle"`` (Shift: what is in goes out, what is
        out comes in) and ``"remove"`` (Shift+Ctrl). ``additive=True`` is
        the old spelling of ``"add"`` and still works; an explicit *mode*
        wins over it."""
        mode = mode or ("add" if additive else "replace")
        if mode == "replace":
            self.selection.clear()
        if mode == "toggle":
            for ent in edges:
                if ent in self.selection:
                    self.selection.discard(ent)
                else:
                    self.selection.add(ent)
        elif mode == "remove":
            self.selection.difference_update(edges)
        else:
            self.selection.update(edges)
        self.bump_view()

    def clear_selection(self) -> None:
        if self.selection:
            self.selection.clear()
            self.bump_view()

    def bump_view(self) -> None:
        """A change of what is shown, not of the document (the selection):
        the caches keyed on ``version`` refresh, the document stays clean."""
        self.version += 1
        self.view_version += 1

    def bump_sections(self) -> None:
        """The section planes changed and nothing else (see
        ``section_version``): the document is edited, the model is not."""
        self.version += 1
        self.section_version += 1

    @property
    def geometry_version(self) -> int:
        """``content_version`` without the section planes' own bumps: what
        the faces, edges and objects are."""
        return self.version - self.view_version - self.section_version

    @property
    def revision(self) -> int:
        """Monotonic observed document token, independent of renderer
        resets: it moves when ``content_version``, a placement's fields or
        tree, a mesh's revision, the tags or their folders moved since the
        last read (``model/scene_revision.rs``, read from the document)."""
        return ingetrazo_core.scene_revision(self)

    @property
    def content_version(self) -> int:
        """``version`` minus the view-only bumps: what "unsaved changes"
        compares against the version that was saved."""
        return self.version - self.view_version

    def invert_selection(self) -> int:
        """Edit ▸ Invert Selection (Ctrl+Shift+I): select every
        entity of the open context that is NOT selected now, and drop what
        is. The universe is Select All's — the loose edges and faces, the
        context's groups (the model's, or the open group's children), the
        dimensions and, in the model, the construction guides — minus what a click or a box could not pick either:
        hidden objects and faces, hidden or locked layers, and hidden edges
        (a smoothed surface's inner edges) while the hidden-geometry view is
        off. Returns the size of the new selection."""
        ctx = self.edit_group
        groups = self.groups if ctx is None else (getattr(ctx, "children", None) or [])
        show_hidden = bool(self.show_hidden_geometry)
        universe = [e for e in self.edges
                    if self.entity_selectable(e)
                    and (show_hidden or not getattr(e, "hidden", False))]
        universe += [f for f in self.faces if self.entity_selectable(f)]
        universe += [g for g in groups if self.entity_selectable(g)]
        universe += [d for d in self.dimensions if self.entity_selectable(d)]
        if ctx is None:                      # guides belong to the model (#627)
            universe += [g for g in self.guides if self.entity_selectable(g)]
        new = [ent for ent in universe if ent not in self.selection]
        self.selection.clear()
        self.selection.update(new)
        self.bump_view()             # the GL colour caches are keyed on it
        return len(new)

    def clear(self) -> None:
        self.end_group_edit()
        if (self.mesh.edges or self.mesh.faces or self.selection
                or self.groups or self.dimensions or self.georef
                or self.tile_layer or self.geo_paths or self.terrain
                or self.guides or self.geo_points or self.text_labels
                or self.saved_views or self.compositions
                or self.scene_folders
                or self.image_planes or self.plugin_data or self.link_registry
                or self.kept_components or self.section_planes
                or self.materials or self.animation.to_dict()
                or not self.show_section_planes
                or not self.show_section_cuts
                or self.show_hidden_objects or self.show_hidden_geometry
                or self.custom_scales or self.back_face_color is not None
                or len(self.layers) > 1 or self.layer_folders
                or self.active_layer or self.hatch_patterns
                or self.line_types or self.linetype_colors
                or self.bricks or self.north_deg):
            self.mesh.clear()
            self.groups.clear()
            self.dimensions.clear()
            self.text_labels.clear()
            self.text_placement_anchors.clear()
            self.geo_paths.clear()
            self.geo_points.clear()
            self.guides.clear()
            self.image_planes.clear()
            self.saved_views.clear()
            self.scene_folders = []
            self.scene_folder_parents = {}
            self.compositions.clear()
            self.custom_scales.clear()
            self.kept_components.clear()
            self.materials.clear()
            self.plugin_data = {}
            self.link_registry = {}
            self.selection.clear()
            from core.layers import DEFAULT_LAYER, Layer
            self.layers = [Layer(DEFAULT_LAYER)]
            self.active_layer = None
            self.layer_folders = []
            self.folder_parents = {}
            self.folder_visibility = {}
            self.layer_folder_uids = {}
            self.layer_folder_names = {}
            self.hatch_patterns = {}
            self.line_types = {}
            self.linetype_colors = {}
            self.bricks = {}
            self.georef = None
            self.tile_layer = None
            self.terrain = None
            self.back_face_color = None
            self.active_ifc = None
            self.display_style = _make_style()
            self.shadows = _make_shadows()
            self.animation = _make_animation()
            self.section_planes.clear()
            self.show_section_planes = True
            self.show_section_cuts = True
            self.show_hidden_objects = False
            self.show_hidden_geometry = False
            self.show_axes = True
            self.north_deg = 0.0
            self.version += 1
        # Outside the guard: an empty document has a camera to forget too.
        self.camera_home = None
        self.views_home = None
        self.units = {"length": "m", "precision": 2}
        self.plugin_data = {}
        self.link_registry = {}

    # ---- Queries ------------------------------------------------------------

    def selection_bounds(self) -> tuple[QVector3D, QVector3D] | tuple[None, None]:
        """Axis-aligned bounding box of the SELECTION — what Zoom Selection
        frames, as ``bounds()`` is what Zoom Extents frames. ``(None, None)``
        when nothing selected has a place in space.

        Loose edges and faces, whole groups (nested placements included),
        dimensions, reference images and construction guides — by their
        point only: a guide line is infinite, and its ±10 km segment made
        Ctrl+A then Zoom Selection frame a dot (#627). Not cached: it runs
        once per command, over the selection only. In Rust
        (``rust/ingetrazo_core/src/model/scene_bounds.rs``)."""
        return _NATIVE.scene_selection_bounds(self)

    def _bounds_key(self):
        """What :meth:`bounds` depends on besides the content: the edit
        context (it swaps ``mesh``), Hide Similar (it follows the context)
        and the layers' visibility. Every change of these bumps ``version``
        already; the key is there so that a view-only bump — the selection
        — is told apart from them."""
        return (self.content_version,
                tuple(id(level["group"]) for level in self._edit_stack),
                id(self.mesh), bool(self.hide_similar_components),
                tuple((ly.key, ingetrazo_core.tag_edits.effective_visible(self, ly)) for ly in self.layers))

    def bounds(self) -> tuple[QVector3D, QVector3D] | tuple[None, None]:
        """Axis-aligned bounding box of all geometry. ``(None, None)`` if empty.

        CACHED per ``version`` — hovering over empty space derives the work
        plane (and the status-bar coordinate) from the model centre, so this
        runs on EVERY mouse move across sky or base map. Every mutation —
        commands, layer toggles — bumps ``version``, the same invariant the
        viewport's chunk caches key on. A view-only bump (a click that
        selects, ``bump_view``) changes ``version`` but not the box: the
        content key (:meth:`_bounds_key`) keeps the cached box across it.

        Group bounds are read from one vertex array per MESH (a model of
        21 406 placements over 2 722 meshes read 14 million vertices one by
        one, ~7 s, #158), so an orphaned vertex left by a deletion may pad
        them slightly until the weld reuses it; a placement that cannot
        widen the box is not read at all, and a lazy mesh's placed box is
        kept from the last walk (``_bounds_parts``). Loose entities keep
        the exact per-entity visibility walk. In Rust
        (``rust/ingetrazo_core/src/model/scene_bounds.rs``)."""
        return _NATIVE.scene_bounds(self)


# ``_groups_owner``: a placement whose children stand in for the top
# level while a command runs inside an open group (GroupInContextCommand:
# with Python lists it swapped ``scene.groups`` for the children list).
def _groups(self):
    owner = self.__dict__.get("_groups_owner")
    return (self._root if owner is None else owner).children

def _set_groups(self, value):
    owner = self.__dict__.get("_groups_owner")
    (self._root if owner is None else owner).children = value

def _tags(self):
    return self._root._tags

def _set_tags(self, value):
    self._root._tags = value

Scene.groups = property(_groups, _set_groups, doc="The root placements.")
Scene.layers = property(_tags, _set_tags, doc="The tags, in panel order.")

def _root_table(name, track):
    """A scene table kept on the scene's root node as a document
    attribute (slice 2): the tag folders and their tables, the active
    tag, plugin data. A dict or a list reads as a tracked container
    that writes back when edited in place (core/doc_model.py)."""
    def get(self):
        return self._root._attr(name, track)

    def put(self, value):
        self._root._set_attr(name, value)
    return property(get, put, doc=f"``{name}``, kept in the document.")

#: The scene fields that live on the root node (their dataclass
#: defaults still apply: ``__init__`` assigns them through these).
ROOT_TABLES = ("units", "active_layer", "layer_folders", "folder_parents",
               "folder_visibility", "layer_folder_uids",
               "layer_folder_names", "plugin_data", "link_registry",
               "dimensions", "text_labels", "text_placement_anchors", "dimension_style", "saved_views",
               "scene_folders", "scene_folder_parents", "compositions",
               "guides", "image_planes", "geo_points", "geo_paths",
               "custom_scales", "section_planes", "show_section_planes",
               "show_section_cuts")
for _name in ROOT_TABLES:
    setattr(Scene, _name, _root_table(_name, _name != "active_layer"))
del _name

def _materials(self):
    return self._root._materials

def _set_materials(self, value):
    self._root._materials = value

def _kept(self):
    return self._root._kept

def _set_kept(self, value):
    self._root._kept = value

Scene.materials = property(_materials, _set_materials,
                           doc="The materials registry, by name.")
Scene.kept_components = property(_kept, _set_kept,
                                 doc="Kept component definitions.")


