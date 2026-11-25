# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Layer / tag system: visibility and locking (tags).

A layer never *owns* geometry — it is a label an entity carries (faces via
``attrs["layer"]``, edges via their ``layer`` slot, groups via ``.layer``).
Hiding a layer removes its entities from render, picking and box selection;
locking keeps them visible but unpickable. Everything unlabelled lives on the
default layer, which always exists and cannot be removed.

This is what makes the '2D that emerges' workflow real: structure / walls /
plumbing / furniture live on their own layers of one model, and a top view in
parallel projection with the right layers on IS the plan drawing.
"""
from __future__ import annotations

from core.group import new_uid
import ingetrazo_core as _rs
from ingetrazo_core import TAG_PALETTE as PALETTE, automatic_tag_color

DEFAULT_LAYER = "Layer 0"

# Where an imported photogrammetric survey lands (Track G, G6). Its own layer
# by default, not the default one: the whole point of importing a survey is to
# draw on top of it, and you need to switch it off to look at what you drew —
# which would take your model with it if they shared a layer. A plain stored
# name like DEFAULT_LAYER, not a translated one: layer names are document data
# and must not change with the interface language.
SURVEY_LAYER = "Survey"


#: The default layer's own colour: the plain model white, so what nobody
#: sorted onto a layer reads as "not classified".
DEFAULT_LAYER_COLOR = (0.96, 0.95, 0.925)

#: Line types a layer can ink its edges with, and their dash pattern in
#: millimetres on paper (dash, gap, …; empty = continuous), with their
#: names in a DXF — the ones every CAD program ships in its standard line
#: type file. The catalogue, the ``.lin`` files and the dash layout live in
#: Rust (``rust/ingetrazo_core/src/linetypes.rs``); the functions below are
#: their Python names.
LINETYPES = {key: tuple(pattern) for key, pattern, _dxf in _rs.linetype_table()}
DXF_LINETYPES = {key: dxf for key, _pattern, dxf in _rs.linetype_table()}

#: The key prefix of a line type the user made or brought in from a line
#: type file (``.lin``), kept in the document (``Scene.line_types``):
#: ``"lin:<name>"``.
CUSTOM_LINETYPE = "lin:"


def known_linetype(key) -> bool:
    """Whether ``key`` names a line type: a built-in one or a custom one."""
    return key in LINETYPES or (isinstance(key, str)
                                and key.startswith(CUSTOM_LINETYPE)
                                and len(key) > len(CUSTOM_LINETYPE))


# ---- Styles: Rust ------------------------------------------------------------
# The tag styles are ``ingetrazo_core.tag_styles``
# (``rust/ingetrazo_core/src/model/tag_styles.rs``): line types by document,
# colours and looks, sheets' line styles and hatches. Callers import them from
# there; the two plugins call (the legend's rows, what does not print) stay
# here by name.
legend_rows = _rs.tag_styles.legend_rows
unprinted = _rs.tag_styles.unprinted


def text_width(text: str, size: float) -> float:
    """About how wide ``text`` runs at ``size`` mm high, in mm — what a
    line type leaves free for it (a sans serif, capitals mostly)."""
    return _rs.linetype_text_width(text, float(size))


def clean_line_text(entry, pattern) -> dict | None:
    """The text of a line type entry, read defensively: ``{"text",
    "text_size", "text_at"}`` — the words, their height (paper mm) and the
    piece of ``pattern`` they sit in, centred (a gap); ``None`` without
    one."""
    return _rs.linetype_clean_line_text(entry, pattern)


def with_text(pattern, text: str, size: float = 2.5) -> tuple:
    """``pattern`` (pieces, dash > 0, gap < 0) with ``text`` in it: room
    as wide as the words and a margin each side, added to its last gap (or
    as a gap of its own) — the text sits centred in that gap. Returns
    ``(pattern, {"text", "text_size", "text_at"})``; the pattern alone when
    there is no text."""
    return _rs.linetype_with_text(pattern, text, float(size))


def text_offset(pattern, at: int) -> float:
    """Where along one period of the drawn pattern (:func:`drawn_dashes`,
    which moves a leading gap to the end) the middle of piece ``at`` of
    ``pattern`` falls, in mm."""
    return _rs.linetype_text_offset(pattern, int(at))


#: The model draws a line type's dashes as a sheet at 1:50 does — metres
#: of model per paper millimetre.
MODEL_DASH_SCALE = 0.05


def dash_segments(segs, pattern, unit: float = MODEL_DASH_SCALE):
    """``segs`` ((E, 2, 3) world) cut as the dash ``pattern`` (paper mm:
    dash, gap, dash…; a dot is a dash of 0) at ``unit`` metres per paper
    mm, restarting on each segment as a pen does: ``(dashes, gaps)``, two
    (K, 2, 3) float32 arrays. A dot is drawn as a short dash. Too fine a
    pattern for a long edge (over 400 periods) leaves it whole."""
    return _rs.linetype_dash_segments(segs, tuple(pattern or ()), float(unit))


def text_marks(p0, p1, spec) -> list:
    """Where a line type's text goes along the line ``p0``→``p1`` (paper mm,
    the pattern restarting at ``p0`` as a pen restarts it on each line):
    ``[(x, y, degrees)]``, the middle of each word that fits whole, turned
    along the line and kept reading upright. ``spec``: :func:`linetype_text`."""
    return _rs.linetype_text_marks(p0, p1, spec)


def drawn_dashes(pieces) -> tuple:
    """Pieces as a line type file has them (dash > 0, gap < 0, dot 0) as
    a pen draws them: dash, gap, dash, gap… in turn, all ≥ 0, starting with
    a dash — two pieces of a kind in a row joined, a leading gap moved to
    the end."""
    return tuple(_rs.linetype_drawn_dashes(pieces))


def file_dashes(drawn) -> tuple:
    """:func:`drawn_dashes` the other way: dash, −gap, … for a file."""
    return tuple(_rs.linetype_file_dashes(drawn))


def clean_pattern(raw) -> list | None:
    """A dash pattern as typed or read from a file, kept sane: finite
    numbers in mm, at most a few dozen pieces, something drawn and
    something left blank; ``None`` when there is no such pattern."""
    return _rs.linetype_clean_pattern(raw)


def parse_lin(text: str) -> tuple[dict, list]:
    """The line types of a line type file (``.lin``) — the plain text
    format of drawing programs: ``*NAME,description`` and then
    ``A,dash,-gap,0,…``. Returns ``({name: {"label", "pattern"[, "text",
    "text_size", "text_at"]}}, skipped)``: the line types in mm — with
    their text, when a piece writes one (``["GAS",STANDARD,S=2.5,…]``: the
    two gaps around it made one, the text centred in it) — and the names
    of those that carry a shape (``[NAME,file.shx,…]``), left out. A
    pattern whose every piece is a fraction of a unit is taken as inches
    (the imperial files) and turned into mm."""
    return _rs.linetype_parse_lin(text)


def to_lin(entries) -> str:
    """A line type file (``.lin``, mm) of ``entries`` — ``[(name, label,
    pattern[, text spec])]`` — that :func:`parse_lin` and drawing programs
    read. A text goes in the middle of its gap, as the format writes one:
    ``["GAS",STANDARD,S=2.5,R=0.0,X=-…,Y=-…]`` between its two halves."""
    return _rs.linetype_to_lin(entries)


#: How much a locked layer's faces wash toward white: drawn, but plainly
#: not in play.
LOCKED_FADE = 0.55


class _LayerMethods:
    """A tag with display state.

    Entities carry the tag's ``key`` — an identity that never changes —
    and ``name`` is only its label: free to repeat (twenty «Beams», one per
    folder) and to rename without relabelling anything (#tags-by-key). The
    key defaults to the name the tag is made with, so a document written
    before keys existed — where names were the identity — reads unchanged.

    ``color`` is an RGB tuple (0–1) or ``None`` for the automatic one (see
    :func:`layer_color`); ``folder`` is the KEY of the folder the layer is
    filed under in the Layers panel, or ``None``. Folder visibility gates
    the tag without changing its own visibility flag."""

    __slots__ = ()

    def __init__(self, name: str, *args, **kwargs) -> None:
        """``Layer(name, visible=True, locked=False, color=None, folder=None,
        linetype="continuous", lineweight=None, opacity=1.0, printable=True,
        uid=None, key=None, cut_weight=None, hatch=None, hatch_ground=None,
        hatch_color=None, hatch_angle=0.0, hatch_scale=1.0,
        hatch_on_paper=None, brick=None, hatch_cross=False, joints=None,
        mortar=False, hatch_offset=None)``: every value cleaned
        (``model/tag_styles.rs: tag_init``). ``key`` is what entities carry,
        stable across renames (the name by default); ``color`` ``None`` is
        the automatic one; ``linetype`` one of :data:`LINETYPES` or the
        document's ``lin:…``; ``lineweight`` / ``cut_weight`` pen mm (``None``:
        the sheet's); ``hatch`` one of ``core.hatches.HATCH_LABELS``;
        ``hatch_ground`` / ``hatch_color`` ``None``, ``"layer"`` or an RGB;
        ``hatch_on_paper`` ``True`` keeps the paper size at any scale;
        ``brick`` ``(length, width, height, joint)`` mm; ``joints`` one of
        ``core.hatches.JOINTS``; ``opacity`` 0.1–1; ``printable`` ``False``
        leaves the tag out of the sheets."""
        _rs.tag_styles.tag_init(self, name, *args, **kwargs)

    def to_dict(self) -> dict:
        """The tag as a file stores it: defaults left out (the document's
        own archive, ``file_styles.rs``)."""
        return _rs.igz_archive_layer(self)

    def has_style(self) -> bool:
        """Whether anything beyond the name and visibility is set — what
        makes a document worth writing its layer list for."""
        return (self.color is not None or bool(self.folder)
                or self.linetype != "continuous"
                or self.lineweight is not None or self.opacity < 1.0
                or not self.printable or self.cut_weight is not None
                or self.hatch is not None or self.hatch_ground is not None
                or self.hatch_color is not None or bool(self.hatch_angle)
                or abs(self.hatch_scale - 1.0) > 1e-9
                or self.hatch_offset is not None
                or self.hatch_on_paper is not None
                or self.brick is not None or self.hatch_cross
                or self.joints is not None or self.mortar)

    @classmethod
    def from_dict(cls, raw: dict) -> "Layer":
        """A tag from its stored form, read defensively (``file_styles.rs``)."""
        return _rs.igz_load_layer(raw, cls)


from core import doc_model as _dm

#: A tag's fields that live in the document (core/doc_model.py) when it
#: is Rust: its core fields, and (slice 2) its sheet styles, as document
#: attributes (plain values: tuples, numbers, strings).
CORE_TAG_FIELDS = ("uid", "name", "key", "visible", "locked", "color", "folder")
STYLE_FIELDS = ("linetype", "lineweight", "cut_weight", "hatch",
                "hatch_ground", "hatch_color", "hatch_angle", "hatch_scale",
                "hatch_offset", "hatch_on_paper", "brick", "hatch_cross",
                "joints", "mortar", "opacity", "printable")
TAG_FIELDS = CORE_TAG_FIELDS + STYLE_FIELDS

class Layer(_LayerMethods, _dm.NATIVE.Tag):
    __doc__ = _LayerMethods.__doc__

    def __reduce__(self):
        state = dict(vars(self))
        state.update(_dm.handle_state(self, TAG_FIELDS))
        return (_dm.new_handle, (type(self),), state)

    __setstate__ = _dm.set_handle_state

for _name in STYLE_FIELDS:
    setattr(Layer, _name, _dm.attribute(_name, track=False))
del _name


def layer_values(ly) -> dict:
    """Every field of a tag by name — what ``vars(ly)`` gave when all of
    them were instance attributes (in the order ``__init__`` sets them)."""
    out = dict(vars(ly))
    doc = {name: getattr(ly, name) for name in _INIT_ORDER}
    doc.update(out)
    return doc


#: The order ``Layer.__init__`` sets its fields in.
_INIT_ORDER = ("uid", "name", "key", "visible", "locked", "color", "folder",
               "linetype", "lineweight", "cut_weight", "hatch",
               "hatch_ground", "hatch_color", "hatch_angle", "hatch_scale",
               "hatch_offset", "hatch_on_paper", "brick", "hatch_cross",
               "joints", "mortar", "opacity", "printable")


#: ``Layer.hatch_ground`` / ``hatch_color`` for «the layer's own colour»
#: (light, for a ground; as it is, for the ink) — by layer.
GROUND_LAYER = "layer"


#: The tag a linked file's untagged content wears in the host
#: (``ingetrazo_core.link_tag_name``): «<file> / Untagged». It stands for the
#: source's default tag, so for colour it reads as none — the piece takes
#: its owner's tag, as it did in its own file.
LINK_UNTAGGED_SUFFIX = " / Untagged"


def ordered_names(scene) -> list[str]:
    """The layer names in the order the Layers tab lists them with «Sort by
    order created», folders unfolded: the default layer first, then each
    folder and every layer out of one as they were made — folders before
    layers, as the tab ranks them — with what a folder holds inside it, its
    subfolders first (``rust/ingetrazo_core/src/tag_folders.rs``).

    Ordering the Browser by layer follows this list, so the rows on the
    layers of a folder stay together, in the folder's place. The model is
    not touched: this is how the tab reads it."""
    layers = list(getattr(scene, "layers", None) or ())
    return _rs.tag_ordered_keys([(ly.key, ly.folder) for ly in layers],
                                folders_of(scene),
                                getattr(scene, "folder_parents", None) or {},
                                DEFAULT_LAYER)


def taggable(entities) -> list:
    """The entities a layer is assigned to — faces, edges, groups and
    components, dimensions, leader texts, section planes, guides and
    reference images. The one list Entity Info, the right-click, the
    Layers panel and its toolbar all ask."""
    from core.dimension import Dimension
    from core.group import Group
    from core.guide import Guide, GuideCircle
    from core.image_plane import ImagePlane
    from ingetrazo_core.mesh import Edge, Face
    from core.section import SectionPlane
    from core.textlabel import TextLabel
    kinds = (Face, Edge, Group, Dimension, TextLabel, SectionPlane, Guide,
             GuideCircle, ImagePlane)
    return [e for e in entities or () if isinstance(e, kinds)]


def selection_layers(scene) -> set[str]:
    """The layers what is selected sits on (empty with nothing selected) —
    what the Layers panel and the Layers toolbar highlight."""
    return {layer_of(e) for e in taggable(getattr(scene, "selection", None))}


#: The key a layer template file is recognised by.
TEMPLATE_KEY = "ingetrazo_layers"


# ---- Folders, display state, templates: Rust -------------------------------
# ``ingetrazo_core.tag_edits`` (``rust/ingetrazo_core/src/model/tag_edits.rs``):
# the folder tree and its edits, the labels, the display state (visible,
# locked, isolated, active), templates and line type files. Callers import
# them from there; the folder queries and the active tag, which plugins and
# scripting use, stay here by name.
_te = _rs.tag_edits
folders_of = _te.folders_of
folder_parent = _te.folder_parent
layers_in_folder = _te.layers_in_folder
active_layer = _te.active_layer
subfolders = _te.tag_scene_subfolders


def object_ids(scene, *, include_groups=True) -> set:
    """Identity of every object in the document — groups and components at
    any depth (by ``id`` and by ``uid``), dimensions, leader texts,
    guides. Compared before and after a command to find what it created
    (:func:`tag_new_objects`). Geometry-only commands pass
    ``include_groups=False``: no group can be created by those edits."""
    from ingetrazo_core import purge_iter_groups as iter_groups
    ids: set = set()
    groups = (getattr(scene, "groups", None) or ()) if include_groups else ()
    for g in iter_groups(groups):
        ids.add(id(g))
        if getattr(g, "uid", None):
            ids.add(g.uid)
    for coll in ("dimensions", "text_labels", "guides"):
        ids.update(id(o) for o in getattr(scene, coll, None) or ())
    return ids


#: The layers guides, dimensions and leader texts go to by themselves while
#: no layer is active: ``(scene collection, English name)``. The name is
#: translated when the layer is made, so a Spanish document gets «Guías»,
#: «Cotas» and «Textos».
AUTO_LAYERS = (("guides", "Guides"), ("dimensions", "Dimensions"),
               ("text_labels", "Texts"))


def auto_ids(scene) -> set:
    """Identity of the guides, dimensions and texts in the document — the
    cheap «before» of a command run with no active layer
    (:func:`tag_new_objects` then only looks at those lists)."""
    ids: set = set()
    for coll, _name in AUTO_LAYERS:
        ids.update(id(o) for o in getattr(scene, coll, None) or ())
    return ids


def auto_layer(scene, english: str) -> str:
    """The automatic layer for ``english`` («Guides», «Dimensions»,
    «Texts») in the interface language, made the first time it is needed.
    One the user hid or locked hands back to the default layer: nothing
    new is ever drawn straight into hiding, as with the active layer.
    «Guides» is born not printed: guides never reach a sheet, and a printed
    layer would stand in every sheet's layer legend."""
    from core.i18n import tr
    name = tr(english)
    ly = scene.layer(name)
    if ly is None:
        scene.layers.append(Layer(name, printable=english != "Guides"))
        return name
    if not ly.visible or ly.locked:
        return DEFAULT_LAYER
    return ly.key


def _tag_automatic(scene, before: set) -> int:
    """With no layer active, the new guides go onto «Guides», the new
    dimensions onto «Dimensions» and the new texts onto «Texts»
    (scaffolding and annotation are what is switched off all at once);
    everything else stays where it was."""
    n = 0
    for coll, english in AUTO_LAYERS:
        fresh = [o for o in getattr(scene, coll, None) or ()
                 if id(o) not in before and layer_of(o) == DEFAULT_LAYER]
        if not fresh:
            continue
        name = auto_layer(scene, english)
        if name == DEFAULT_LAYER:
            continue
        for obj in fresh:
            assign_layer(obj, name)
            n += 1
    return n


def tag_new_objects(scene, before: set, *, include_groups=True, tagged_groups=None) -> int:
    """Put the objects a command created on the active layer.

    A group, a component, a dimension, a leader text or a guide made while
    a layer is active goes onto it; loose lines and faces stay on the default
    layer (a layer hidden never leaves half a solid). Only the outermost
    new object is tagged: a new group's own children follow it. One that
    already carries a layer (a copy, a pasted object) keeps its own. With
    no layer active, guides, dimensions and texts go to their automatic
    layers (:data:`AUTO_LAYERS`) — ``before`` is then :func:`auto_ids`.
    Returns how many objects were tagged."""
    from ingetrazo_core import purge_iter_groups as iter_groups
    name = active_layer(scene)
    if name == DEFAULT_LAYER:
        return _tag_automatic(scene, before)
    # A copy that REPLACES an object (Make Unique, Replace Component…)
    # keeps its uid: that one is not new, and keeps the layer it had.
    groups = (getattr(scene, "groups", None) or ()) if include_groups else ()
    fresh = [g for g in iter_groups(groups)
             if id(g) not in before
             and getattr(g, "uid", None) not in before]
    # Only the outermost new objects: a new group's own children keep
    # the default layer and follow their container.
    inner = {id(c) for g in fresh for c in getattr(g, "children", None) or ()}
    fresh = [g for g in fresh if id(g) not in inner]
    for coll in ("dimensions", "text_labels", "guides"):
        fresh += [o for o in getattr(scene, coll, None) or ()
                  if id(o) not in before]
    n = 0
    for obj in fresh:
        if layer_of(obj) == DEFAULT_LAYER:
            assign_layer(obj, name)
            if tagged_groups is not None and hasattr(obj, "mesh"):
                tagged_groups.append(obj)
            n += 1
    return n


def layer_of(entity) -> str:
    """The layer name an entity carries (default when unlabelled)."""
    attrs = getattr(entity, "attrs", None)
    if attrs is not None:                          # Face
        return attrs.get("layer") or DEFAULT_LAYER
    return getattr(entity, "layer", None) or DEFAULT_LAYER


def assign_layer(entity, name: str) -> None:
    """Label an entity with a layer (default name clears the label)."""
    value = None if name == DEFAULT_LAYER else name
    attrs = getattr(entity, "attrs", None)
    if attrs is not None:                          # Face
        if value is None:
            attrs.pop("layer", None)
        else:
            attrs["layer"] = value
    elif hasattr(entity, "layer"):
        entity.layer = value


def rename_layer(scene, old_name: str, new_name: str) -> bool:
    """Give the tag ``old_name`` (its key, or its label) the label
    ``new_name``. Only the label changes: everything carries the tag's key,
    which stays, so nothing is relabelled — geometry, kept definitions,
    the active tag and the scenes that hide it all follow by themselves.
    Two tags may share a label («Beams» in every folder).

    Not an undo step: renaming is easy to redo by hand, and Ctrl+Z after
    drawing should take back the drawing, not a name typed in between.
    Returns False (and changes nothing) for the default layer, an empty
    name, the default layer's own name, or a layer that does not exist."""
    ly = scene.layer(old_name)
    if (ly is None or ly.key == DEFAULT_LAYER or not new_name
            or not new_name.strip() or new_name == DEFAULT_LAYER):
        return False
    ly.name = new_name
    return True


def grain_layers(scene) -> list:
    """The layers whose timber is crossed where it is cut across its grain
    (``Layer.hatch_cross``): for them a view measures how far each cut piece
    runs through the plane."""
    from core.hatches import WOOD
    return [ly.key for ly in getattr(scene, "layers", ())
            if getattr(ly, "hatch_cross", False)
            and getattr(ly, "hatch", None) in WOOD]


def section_cut_kind(scene):
    """What kind of cut a view of ``scene`` shows: ``"plan"`` (the active
    section plane lies flat), ``"vertical"`` (it stands), ``None`` (no
    section) — masonry is drawn one way in a plan, another in a section."""
    plane = (scene.active_section() if hasattr(scene, "active_section")
             else None)
    if plane is None:
        return None
    return "plan" if abs(plane.normal.z()) >= 0.7 else "vertical"
