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


def linetype_pattern(key, scene=None) -> tuple:
    """The dash pattern of line type ``key`` as it is drawn: paper mm,
    dash, gap, dash, gap… in turn (as :data:`LINETYPES` holds them; a dot
    is a dash of 0; empty: continuous) — a built-in one, or the document's
    own, whose stored pieces (dash > 0, gap < 0, dot 0) are turned so."""
    if key in LINETYPES:
        return tuple(LINETYPES[key])
    if isinstance(key, str) and key.startswith(CUSTOM_LINETYPE):
        entry = (getattr(scene, "line_types", None) or {}).get(
            key[len(CUSTOM_LINETYPE):])
        if isinstance(entry, dict):
            return drawn_dashes(entry.get("pattern") or ())
    return ()


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


def linetype_text(key, scene=None):
    """The text of line type ``key`` as it is drawn: ``(text, height mm,
    where its middle falls along a period, the period)`` in paper mm;
    ``None`` for a line type without one (every built-in one)."""
    if not (isinstance(key, str) and key.startswith(CUSTOM_LINETYPE)):
        return None
    entry = (getattr(scene, "line_types", None) or {}).get(
        key[len(CUSTOM_LINETYPE):])
    pattern = entry.get("pattern") if isinstance(entry, dict) else None
    spec = clean_line_text(entry, pattern)
    if spec is None:
        return None
    period = sum(drawn_dashes(pattern))
    return (spec["text"], spec["text_size"],
            text_offset(pattern, spec["text_at"]), period)


def clean_line_colour(raw) -> tuple | None:
    """A line type's own colour as it is kept: ``(r, g, b)`` 0–1, or
    ``None`` (by layer) for none or anything else."""
    if not isinstance(raw, (list, tuple)) or len(raw) != 3:
        return None
    try:
        rgb = tuple(min(1.0, max(0.0, float(c))) for c in raw)
    except (TypeError, ValueError):
        return None
    return rgb if all(c == c for c in rgb) else None


def linetype_color(key, scene=None) -> tuple | None:
    """The colour line type ``key`` inks its lines with whatever their
    layer — a gas line yellow, a water main blue — or ``None``: the
    layer's. A built-in one takes the colour the document gave it."""
    if key in LINETYPES:
        return clean_line_colour(
            (getattr(scene, "linetype_colors", None) or {}).get(key))
    if not (isinstance(key, str) and key.startswith(CUSTOM_LINETYPE)):
        return None
    entry = (getattr(scene, "line_types", None) or {}).get(
        key[len(CUSTOM_LINETYPE):])
    return clean_line_colour(entry.get("color")) \
        if isinstance(entry, dict) else None


def set_linetype_color(scene, key, rgb) -> bool:
    """Give line type ``key`` the colour ``rgb`` — ``None``: by layer again
    — a built-in one in the document's list, an own one in its entry.
    Returns whether anything changed."""
    rgb = clean_line_colour(rgb)
    if linetype_color(key, scene) == rgb:
        return False
    if key in LINETYPES:
        colours = dict(getattr(scene, "linetype_colors", None) or {})
        if rgb is None:
            colours.pop(key, None)
        else:
            colours[key] = rgb
        scene.linetype_colors = colours
        return True
    if not (isinstance(key, str) and key.startswith(CUSTOM_LINETYPE)):
        return False
    own = dict(getattr(scene, "line_types", None) or {})
    entry = own.get(key[len(CUSTOM_LINETYPE):])
    if not isinstance(entry, dict):
        return False
    entry = dict(entry)
    if rgb is None:
        entry.pop("color", None)
    else:
        entry["color"] = rgb
    own[key[len(CUSTOM_LINETYPE):]] = entry
    scene.line_types = own
    return True


def load_linetype_colors(scene, raw) -> None:
    """The built-in line types' colours as a file keeps them, read
    defensively: known keys and real colours only."""
    if not isinstance(raw, dict):
        return
    colours = dict(getattr(scene, "linetype_colors", None) or {})
    for key, value in raw.items():
        rgb = clean_line_colour(value)
        if key in LINETYPES and rgb is not None:
            colours[key] = rgb
    scene.linetype_colors = colours


def model_line_styles(scene) -> dict:
    """``{layer key: (rgb or None, dash pattern)}`` of the layers whose
    edges the model draws with their line type (the style's «Line types in
    the model»): dashed (the pattern in paper mm, as
    :func:`linetype_pattern` gives it), in a colour of its own, or both.
    A continuous line type by layer is not here: the model draws it as
    ever."""
    out = {}
    for ly in getattr(scene, "layers", ()):
        rgb = linetype_color(ly.linetype, scene)
        pattern = tuple(linetype_pattern(ly.linetype, scene))
        if rgb is not None or pattern:
            out[ly.key] = (rgb, pattern)
    return out


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

    def __init__(self, name: str, visible: bool = True,
                 locked: bool = False, color=None,
                 folder: str | None = None, linetype: str = "continuous",
                 lineweight: float | None = None,
                 opacity: float = 1.0, printable: bool = True, uid: str | None = None,
                 key: str | None = None,
                 cut_weight: float | None = None,
                 hatch: str | None = None, hatch_ground=None,
                 hatch_color=None, hatch_angle: float = 0.0,
                 hatch_scale: float = 1.0,
                 hatch_on_paper: bool | None = None,
                 brick=None, hatch_cross: bool = False, joints=None,
                 mortar: bool = False, hatch_offset=None) -> None:
        self.uid = uid or new_uid()
        self.name = name
        #: What entities carry; stable across renames.
        self.key = key or name
        self.visible = visible
        self.locked = locked
        self.color = tuple(color) if color is not None else None
        self.folder = folder or None
        #: How the layer's edges are inked on a sheet and in a DXF:
        #: one of :data:`LINETYPES`.
        self.linetype = (linetype if known_linetype(linetype)
                         else "continuous")
        #: Pen width on a sheet, in millimetres (``None``: the sheet's own
        #: pen for the line's class).
        self.lineweight = lineweight
        #: Pen width of the layer's section cut on a sheet, in millimetres
        #: (``None``: the sheet's cut pen) — a wall cut heavier than the
        #: door and the window cut beside it, whatever its other lines.
        self.cut_weight = cut_weight
        #: What its section cut is filled with on a sheet: one of
        #: ``core.hatches.HATCH_LABELS`` (brick, concrete, insulation…), or
        #: ``None`` for the frame's own fill.
        from core.hatches import known
        self.hatch = hatch if hatch is not None and known(hatch) else None
        #: A solid colour under the hatch: ``None`` (the paper), ``"layer"``
        #: (the layer's own colour, light) or an RGB tuple (0–1) — white
        #: or grey under brick, say.
        self.hatch_ground = _ground(hatch_ground)
        #: The ink of the hatch's lines: ``None`` (as the frame says — its
        #: fill colour, or the layer's when it fills by layer), ``"layer"``
        #: (by layer: always the layer's colour) or an RGB tuple (a fixed
        #: colour, black say, whatever the frame).
        self.hatch_color = _ground(hatch_color)
        #: The hatch turned this many degrees, and this many times its size.
        self.hatch_angle = _finite(hatch_angle, 0.0) % 360.0
        self.hatch_scale = min(20.0, max(0.05, _finite(hatch_scale, 1.0)))
        #: Where a pattern of pieces (core.hatches.PLACED) starts: ``(x,
        #: y)`` real mm it is moved by — the first joint along a wall, the
        #: first course up it, tiles off an axis — ``None`` for none.
        self.hatch_offset = clean_offset(hatch_offset)
        #: ``True``: the hatch measures the same on paper at any scale
        #: (a drawing's convention); ``False``: it grows and shrinks with the
        #: frame's scale, like the real bricks or tiles it stands for (its
        #: own size is the one at 1:100).
        self.hatch_on_paper = (None if hatch_on_paper is None
                               else bool(hatch_on_paper))
        #: The brick its brickwork hatch is drawn with: ``(length, width,
        #: height, joint)`` in mm, ``None`` for the usual metric one.
        from core.hatches import clean_brick
        self.brick = clean_brick(brick) if brick is not None else None
        #: A wood hatch drawn with a cross from corner to corner where a
        #: piece is cut across its grain (core.hatches.cut_across): studs,
        #: joists and beams in section; the rest in the hatch.
        self.hatch_cross = bool(hatch_cross)
        #: How its masonry joints are drawn: one of core.hatches.JOINTS, or
        #: ``None`` — by the sheet's scale. ``mortar``: the double joints
        #: filled in light grey.
        from core.hatches import JOINTS
        self.joints = joints if joints in JOINTS else None
        self.mortar = bool(mortar)
        #: Face opacity, 0.1–1: a reference layer seen through.
        self.opacity = min(1.0, max(0.1, float(opacity)))
        #: ``False``: seen in the model, left out of the sheets — their
        #: views, PDF and DXF (references, construction lines, notes).
        self.printable = bool(printable)

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


def _finite(raw, default: float) -> float:
    try:
        value = float(raw)
    except (TypeError, ValueError):
        return default
    import math
    return value if math.isfinite(value) else default


#: ``Layer.hatch_ground`` / ``hatch_color`` for «the layer's own colour»
#: (light, for a ground; as it is, for the ink) — by layer.
GROUND_LAYER = "layer"


def _ground(raw):
    """A hatch ground or ink as stored: ``None``, :data:`GROUND_LAYER` or
    an RGB tuple kept to 0–1; anything else is ``None``."""
    if raw == GROUND_LAYER:
        return GROUND_LAYER
    if isinstance(raw, (list, tuple)) and len(raw) >= 3:
        try:
            return tuple(min(1.0, max(0.0, float(c))) for c in raw[:3])
        except (TypeError, ValueError):
            return None
    return None


def hatch_ground_rgb(ly) -> tuple | None:
    """The colour under a layer's hatch: its own pick, the layer's colour
    made light for :data:`GROUND_LAYER`, ``None`` for the paper."""
    ground = getattr(ly, "hatch_ground", None)
    if ground == GROUND_LAYER:
        return light_layer_color(ly)
    return tuple(ground) if ground is not None else None


def hatch_ink_rgb(ly) -> tuple | None:
    """The ink of a layer's hatch lines when the layer sets one: its colour
    by layer (a shade darker, so mid tones read on paper) or the fixed one;
    ``None`` — as the frame says."""
    ink = getattr(ly, "hatch_color", None)
    if ink == GROUND_LAYER:
        return tuple(c * 0.7 for c in layer_color(ly))
    return tuple(ink) if ink is not None else None


def light_layer_color(ly) -> tuple:
    """The layer's colour three quarters of the way to white — a ground a
    hatch still reads on."""
    return tuple(c + (1.0 - c) * 0.75 for c in layer_color(ly))


def layer_color(layer) -> tuple:
    """The colour a layer shows: the picked one, else the default layer's
    white for the default layer, else one from :data:`PALETTE` chosen by the
    name — stable across sessions and machines, and unchanged when other
    layers are added, deleted or sorted."""
    if layer is None:
        return DEFAULT_LAYER_COLOR
    if layer.color is not None:
        return tuple(layer.color[:3])
    if layer.key == DEFAULT_LAYER:
        return DEFAULT_LAYER_COLOR
    return automatic_tag_color(layer.key)


#: The tag a linked file's untagged content wears in the host
#: (``core.links.tag_name``): «<file> / Untagged». It stands for the
#: source's default tag, so for colour it reads as none — the piece takes
#: its owner's tag, as it did in its own file.
LINK_UNTAGGED_SUFFIX = " / Untagged"


def is_link_untagged(name) -> bool:
    """Whether ``name`` is a linked file's stand-in for the default tag."""
    return bool(name) and name.endswith(LINK_UNTAGGED_SUFFIX)


def object_layer(group) -> str:
    """The layer an object's insides take their colour from: its own, else
    the top-level object's it belongs to (a nested piece of a component on
    a layer), else the default layer. ``None`` is the loose model. A linked
    file's «Untagged» counts as none here: with the source's default tag
    renamed on linking, every untagged piece of the facilities model took
    that one tag's colour instead of its owner's (Maxence, 2026-11-09)."""
    for g in (group, getattr(group, "owner", None)):
        name = getattr(g, "layer", None) if g is not None else None
        if name and not is_link_untagged(name):
            return name
    return DEFAULT_LAYER


def layer_paint(scene, group=None) -> dict:
    """The container paint Colour by layer bakes ``group`` with (see
    ``core.materials.effective_attrs``): every layer's colour by name, and
    the object's own layer colour for the faces that carry none."""
    table = {ly.key: layer_color(ly) for ly in getattr(scene, "layers", ())}
    color = table.get(object_layer(group), DEFAULT_LAYER_COLOR)
    return {"by_layer": table, "color": color}


def layer_fx(scene) -> dict:
    """``{name: (opacity, fade)}`` of the layers that change how their faces
    look — see-through ones and locked ones (washed out, so it shows they
    are not in play). Empty when none does: the common case costs nothing."""
    return {ly.key: (ly.opacity, LOCKED_FADE if ly.locked else 0.0)
            for ly in getattr(scene, "layers", ())
            if ly.opacity < 1.0 or ly.locked}


def layer_look(scene, group, base, fx=None):
    """``base`` (the container paint: its own, or :func:`layer_paint`) with
    the layers' look on top, or ``base`` itself when no layer has one."""
    fx = layer_fx(scene) if fx is None else fx
    if not fx:
        return base
    return {"layer_fx": fx, "fx_self": fx.get(object_layer(group)),
            "base": base}


def layer_lines_wanted(scene, style=None) -> bool:
    """Whether a sheet has to ink lines layer by layer: some layer has a
    line type, weight or cut weight of its own, or the style colours by
    layer."""
    style = style if style is not None else getattr(scene, "display_style",
                                                    None)
    if getattr(style, "color_by_layer", False):
        return True
    return any(ly.linetype != "continuous" or ly.lineweight is not None
               or getattr(ly, "cut_weight", None) is not None
               for ly in getattr(scene, "layers", ()))


def line_styles(scene, colored: bool) -> dict:
    """``{name: (rgb or None, linetype, lineweight or None, cut weight or
    None, dash pattern, text)}`` — how each layer's lines are inked on a sheet
    and written to a DXF (the pattern in paper mm, the document's own line
    types resolved). The colour
    only with Colour by layer on (a plan prints black otherwise), and never
    for the default layer, whose white would vanish on paper."""
    out = {}
    for ly in getattr(scene, "layers", ()):
        # A line type with a colour of its own inks with it, on any sheet.
        rgb = linetype_color(ly.linetype, scene) or (
            layer_color(ly) if colored and ly.key != DEFAULT_LAYER
            else None)
        out[ly.key] = (rgb, ly.linetype, ly.lineweight,
                       getattr(ly, "cut_weight", None),
                       linetype_pattern(ly.linetype, scene),
                       linetype_text(ly.linetype, scene))
    return out


def legend_rows(scene) -> tuple[list, dict]:
    """What a sheet's layer legend shows: the visible, printed layers'
    labels — their folder path, so two «Beams» read apart — and, for each,
    its swatch — colour and line type."""
    rows, swatches = [], {}
    hatch_forms = cut_hatch_forms(scene)
    for ly in getattr(scene, "layers", ()):
        if not effective_visible(scene, ly) or not getattr(ly, "printable", True):
            continue
        label = path_label(scene, ly.key)
        rows.append(label)
        swatch = {"color": [round(c, 4) for c in layer_color(ly)],
                  "linetype": ly.linetype}
        swatches[label] = swatch
        if ly.linetype.startswith(CUSTOM_LINETYPE):
            swatch["pattern"] = list(linetype_pattern(ly.linetype, scene))
        if getattr(ly, "hatch", None):
            swatch["hatch_form"] = hatch_forms[ly.key]
            swatch["hatch"] = ly.hatch
            if getattr(ly, "hatch_angle", 0.0):
                swatch["hatch_angle"] = ly.hatch_angle
            families = hatch_forms[ly.key]["families"]
            if families:
                swatch["hatch_families"] = families
            for key, rgb in (("hatch_ground", hatch_ground_rgb(ly)),
                             ("hatch_color", hatch_ink_rgb(ly))):
                if rgb is not None:
                    swatch[key] = [round(c, 4) for c in rgb]
    return rows, swatches


def cut_hatches(scene) -> dict:
    """``{layer key: pattern}`` of the layers whose section cut has a hatch of
    its own (see ``core.hatches``) — empty when none has."""
    return {ly.key: ly.hatch for ly in getattr(scene, "layers", ())
            if getattr(ly, "hatch", None)}


def cut_hatch_grounds(scene) -> dict:
    """``{layer key: rgb}`` — the solid colour under each hatched layer's
    pattern, for the layers that have one."""
    out = {}
    for ly in getattr(scene, "layers", ()):
        rgb = hatch_ground_rgb(ly) if getattr(ly, "hatch", None) else None
        if rgb is not None:
            out[ly.key] = rgb
    return out


def hatch_families(scene, key):
    """The families of an imported pattern (``pat:<name>``) kept in the
    document, ``None`` for a built-in one (or one the document lost)."""
    from core.hatches import IMPORTED
    if not (isinstance(key, str) and key.startswith(IMPORTED)):
        return None
    entry = (getattr(scene, "hatch_patterns", None) or {}).get(
        key[len(IMPORTED):])
    return entry.get("families") if isinstance(entry, dict) else None


def layer_brick(scene, ly):
    """The piece a layer's masonry hatch is drawn with: its own
    (``Layer.brick``), else the document's of that kind (``Scene.bricks``:
    brick, ticholo, block), else ``None`` — the usual one of its kind."""
    own = getattr(ly, "brick", None)
    if own is not None:
        return own
    from core.hatches import piece_kind
    kind = piece_kind(getattr(ly, "hatch", None))
    return (getattr(scene, "bricks", None) or {}).get(kind) if kind else None


def load_bricks(scene, raw) -> None:
    """The document's pieces as a file or a template stores them,
    ``{kind: size}``, read defensively into ``Scene.bricks``."""
    from core.hatches import PIECE_SIZES, clean_brick
    if not isinstance(raw, dict):
        return
    for kind, size in raw.items():
        size = clean_brick(size) if kind in PIECE_SIZES else None
        if size is not None:
            scene.bricks = dict(getattr(scene, "bricks", None) or {},
                                **{kind: size})


def layer_hatch_families(scene, ly):
    """The families a layer's hatch is drawn with when they are not the
    built-in pattern's own: an imported pattern's, or a brick pattern's
    drawn with the layer's brick (its own or the document's). ``None``
    otherwise."""
    key = getattr(ly, "hatch", None)
    families = hatch_families(scene, key)
    brick = layer_brick(scene, ly)
    if families is None and brick is not None:
        from core.hatches import BRICKS, brick_families
        if key in BRICKS:
            return [list(f[:5]) + [list(f[5])]
                    for f in brick_families(key, brick)]
    return families


def hatch_label(scene, key) -> str:
    """A pattern's name as the user knows it (untranslated): the built-in
    label, or an imported pattern's own name."""
    from core.hatches import HATCH_LABELS, IMPORTED
    if key in HATCH_LABELS:
        return HATCH_LABELS[key]
    if isinstance(key, str) and key.startswith(IMPORTED):
        return key[len(IMPORTED):]
    return ""


def hatch_grows(ly) -> bool:
    """Whether a layer's hatch grows with the drawing's scale: as the layer
    says, else as its pattern goes — a material with a real size (brick,
    tiles, boards…) does, a stipple or a generic hatch keeps its size on
    paper."""
    own = getattr(ly, "hatch_on_paper", None)
    if own is not None:
        return not own
    from core.hatches import REAL_SIZE
    return getattr(ly, "hatch", None) in REAL_SIZE


def cut_hatch_forms(scene, scale_n: float = 100.0) -> dict:
    """``{layer key: {"angle", "scale", "families", "brick", "scale_n"}}`` — how
    each hatched layer's pattern is laid on a sheet drawn at 1:``scale_n``:
    turned, sized (on paper; or with the drawing — a material at its real
    size, any other pattern from its size at 1:100), the families of an
    imported pattern or of the layer's own brick, and that brick."""
    out = {}
    for ly in getattr(scene, "layers", ()):
        key = getattr(ly, "hatch", None)
        if not key:
            continue
        size = float(getattr(ly, "hatch_scale", 1.0) or 1.0)
        if hatch_grows(ly):
            from core.hatches import real_scale
            real = real_scale(key, scale_n, layer_brick(scene, ly))
            size *= (real if real is not None
                     else 100.0 / max(1e-6, float(scale_n or 100.0)))
        out[ly.key] = {"angle": float(getattr(ly, "hatch_angle", 0.0) or 0.0),
                        "scale": size,
                        "families": layer_hatch_families(scene, ly),
                        "brick": layer_brick(scene, ly),
                        "cross": bool(getattr(ly, "hatch_cross", False)),
                        "joints": getattr(ly, "joints", None),
                        "mortar": bool(getattr(ly, "mortar", False)),
                        "hatch_scale": float(getattr(ly, "hatch_scale", 1.0)
                                             or 1.0),
                        "offset": getattr(ly, "hatch_offset", None),
                        "scale_n": float(scale_n or 100.0)}
    return out


def clean_offset(raw) -> tuple | None:
    """A pattern's shift as a layer keeps it: ``(x, y)`` real mm, each
    within ±10 m — ``None`` for none or for anything else."""
    import math
    if not isinstance(raw, (list, tuple)) or len(raw) != 2:
        return None
    try:
        x, y = (float(v) for v in raw)
    except (TypeError, ValueError):
        return None
    if not (math.isfinite(x) and math.isfinite(y)) or (not x and not y):
        return None
    return (min(10000.0, max(-10000.0, x)),
            min(10000.0, max(-10000.0, y)))


def cut_hatch_inks(scene) -> dict:
    """``{layer key: rgb}`` — the ink of each hatched layer's pattern, for the
    layers that set one (by layer or fixed)."""
    out = {}
    for ly in getattr(scene, "layers", ()):
        rgb = hatch_ink_rgb(ly) if getattr(ly, "hatch", None) else None
        if rgb is not None:
            out[ly.key] = rgb
    return out


def cut_fill_colours(scene) -> dict:
    """``{layer key: rgb}`` a sheet fills each layer's section cut with when the
    frame asks for the layers' colours: every layer's own colour (picked or
    automatic), except the default layer, which keeps the frame's fill — its
    white would vanish on paper."""
    return {ly.key: layer_color(ly) for ly in getattr(scene, "layers", ())
            if ly.key != DEFAULT_LAYER}


def unprinted(scene) -> list:
    """The layers left out of the sheets."""
    return [ly for ly in getattr(scene, "layers", ())
            if not getattr(ly, "printable", True)]


def hide_unprinted(scene) -> None:
    """Switch off the layers that do not print — for a sheet's render, whose
    caller puts the visibility back."""
    for ly in unprinted(scene):
        ly.visible = False


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
    from core.mesh import Edge, Face
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


# ---- Folders, display state, templates: Rust ---------------------------------
# ``rust/ingetrazo_core/src/model/tag_edits.rs``, step for step on the scene's
# root tables (``layer_folders``, ``folder_parents``, ``layer_folder_names``,
# ``layer_folder_uids``, ``folder_visibility``, ``active_layer``) and its tags.
#
# Folders: ``folders_of`` (the document's list, then any a tag names; each
# given a uid), ``folder_parent`` (a parent that is gone or makes a loop reads
# as the top), ``subfolders``, ``layers_in_folder``, ``folder_visible`` (its
# own switch and its parents'), ``effective_visible``; the labels
# ``folder_label``, ``layer_label``, ``path_label`` («Observation Deck/Steel/
# Beams», for what has to tell tags apart by text) and ``export_labels``;
# ``unique_name``. Edits: ``add_folder``, ``move_folder`` (refused into
# itself or a folder inside it), ``rename_folder`` (only the label: tags and
# subfolders name a folder by its key), ``remove_folder`` (what it held
# moves up), ``move_to_folder`` (the default tag stays out of folders).
#
# Display state: ``active_layer`` (where new objects go: the active tag while
# it shows and is unlocked, else the default one), ``set_visible``,
# ``set_locked``, ``isolate`` (the default tag stays on), ``is_isolated``,
# ``set_active``, ``layer_state`` / ``apply_layer_state``, ``hide_in_scenes``
# (a tag made after the plans were set up, kept out of them).
#
# Templates, an office standard carried between projects: ``template_of``,
# ``template_for`` (the tags a copy carries), ``matched_template`` (identities
# resolved to local names), ``template_name_map``, ``apply_template`` (the
# missing tags added, the others restyled; ``(added, updated)``),
# ``add_missing`` / ``remove_added`` (a paste's tags and their undo), and
# ``load_line_types``.
_te = _rs.tag_edits
for _name in ("folders_of", "folder_parent", "folder_visible", "effective_visible",
              "folder_label", "layer_label", "path_label", "export_labels", "unique_name",
              "layers_in_folder", "add_folder", "move_folder", "rename_folder",
              "remove_folder", "move_to_folder", "active_layer", "set_visible", "set_locked",
              "isolate", "is_isolated", "set_active", "layer_state", "apply_layer_state",
              "hide_in_scenes", "template_of", "matched_template", "template_name_map",
              "apply_template", "template_for", "add_missing", "remove_added",
              "load_line_types"):
    globals()[_name] = getattr(_te, _name)
del _name
subfolders = _te.tag_scene_subfolders
_release_active = _te.tag_release_active


def object_ids(scene, *, include_groups=True) -> set:
    """Identity of every object in the document — groups and components at
    any depth (by ``id`` and by ``uid``), dimensions, leader texts,
    guides. Compared before and after a command to find what it created
    (:func:`tag_new_objects`). Geometry-only commands pass
    ``include_groups=False``: no group can be created by those edits."""
    from core.purge import iter_groups
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
    from core.purge import iter_groups
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
