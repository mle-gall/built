# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Hatch patterns for the section cut on a sheet: what a cut layer is made
of, told by its drawing — brick courses, the stipple of concrete, the waves
of insulation, the dashes of earth.

A pattern is described the way technical drawing has always described one:
families of parallel lines, each at an angle, through an origin, the next
line of the family shifted along itself and across, drawn whole or as a run
of dashes, gaps and dots — all in paper millimetres. A few materials are not
lines (gravel, sand, the stipple of concrete, stones, roof tiles, grass, the
rings of end grain, the waves of insulation and wood): those are generated,
scattered by a hash of their cell on the paper.

The set follows the conventions of architectural and technical sections:
45° hatching for anything generic, the material symbols of the drafting
standards for metals, glass and plastics, and the usual architectural ones
for masonry, concrete, ground, insulation, wood and finishes. Every pattern
here is drawn from those conventions, not taken from any program's file.

Everything is locked to the paper, not to the ring it fills, so a pattern
runs on unbroken across two pieces of the same wall and stays put when the
frame moves. Nothing here draws: :func:`hatch_marks` returns the marks
inside a box and sheets::hatching (Rust) clips and inks them.
"""
from __future__ import annotations

import math
import re

from ingetrazo_core import (fit_band, fitted_marks, generate_hatch, hatch_clip_segments,
                            hatch_fill_quads, masonry_course, masonry_marks)

#: The picker's groups: ``(title, patterns)``, in the order shown. ``None``
#: is not in them: it means «the frame's fill» (solid or 45°, as the frame
#: says) and the picker puts it first.
HATCH_GROUPS = (
    ("General", ("solid", "diagonal", "dashed_diagonal", "double_diagonal",
                 "crosshatch", "grid", "dots", "plus", "hexagon")),
    ("Masonry", ("brick_auto", "brick", "brick_header", "brick_bond",
                 "brick_cross", "brick_flemish", "brick_dutch",
                 "brick_one_half", "brick_on_edge", "brick_rowlock",
                 "brick_honeycomb", "hollow_brick", "perforated_block",
                 "concrete_brick",
                 "fire_brick", "block", "aac_block", "ashlar", "stone",
                 "rubble", "rammed_earth", "tile")),
    ("Concrete and ground", ("concrete", "reinforced", "lean_concrete",
                             "sand", "gravel", "earth", "fill", "clay",
                             "rock")),
    ("Insulation and finishes", ("insulation", "insulation_zigzag",
                                 "rigid_insulation",
                                 "waterproofing", "plaster", "acoustic",
                                 "roof_tile")),
    ("Wood", ("wood", "wood_grain", "wood_open", "wood_flame",
              "wood_diagonal", "wood_boards", "wood_end", "plywood", "osb",
              "clt", "parquet", "wood_herringbone", "wood_cross")),
    ("Metals, glass and plastics", ("steel", "aluminium", "bronze", "lead",
                                    "mesh", "chequer_plate", "glass",
                                    "plastic")),
    ("Vegetation and water", ("grass", "shrubs", "canopy", "crops",
                              "wetland", "water")),
    ("Paving and urban", ("herringbone", "cobblestone", "asphalt",
                          "tactile")),
)

#: Every pattern, in the picker's order.
HATCH_ORDER = tuple(k for _t, keys in HATCH_GROUPS for k in keys)

#: Their names, as the interface shows them (translated by the caller).
HATCH_LABELS = {
    "solid": "Solid",
    "diagonal": "Diagonal hatching",
    "double_diagonal": "Double hatching",
    "crosshatch": "Cross-hatching",
    "grid": "Square grid",
    "dots": "Dots",
    "brick": "Brick",
    "brick_bond": "Brick, English bond",
    "brick_auto": "Brick, bond by wall thickness",
    "brick_header": "Brick, header bond",
    "brick_cross": "Brick, English cross bond",
    "brick_dutch": "Brick, Dutch bond",
    "brick_one_half": "Brick, one and a half bricks thick",
    "brick_on_edge": "Brick on edge, partition",
    "brick_rowlock": "Brick on edge, rowlock",
    "brick_honeycomb": "Brick, honeycomb bond",
    "fire_brick": "Fire brick",
    "block": "Concrete block",
    "aac_block": "Aerated concrete block",
    "perforated_block": "Load-bearing ceramic block (rejillón)",
    "concrete_brick": "Precast concrete brick with holes",
    "stone": "Stone",
    "rubble": "Rubble masonry",
    "tile": "Ceramic tiles",
    "concrete": "Concrete",
    "reinforced": "Reinforced concrete",
    "sand": "Sand, mortar",
    "gravel": "Gravel",
    "earth": "Earth",
    "fill": "Compacted fill",
    "insulation": "Insulation",
    "insulation_zigzag": "Insulation, zigzag",
    "rigid_insulation": "Rigid insulation",
    "roof_tile": "Roof tiles",
    "water": "Water",
    "grass": "Grass",
    "wood": "Wood",
    "wood_end": "Wood, end grain",
    "plywood": "Plywood",
    "parquet": "Parquet",
    "steel": "Steel",
    "aluminium": "Aluminium",
    "bronze": "Bronze, brass",
    "lead": "Lead, zinc",
    "glass": "Glass",
    "plastic": "Plastic, rubber",
    "dashed_diagonal": "Dashed hatching",
    "plus": "Crosses",
    "hexagon": "Hexagons",
    "brick_flemish": "Brick, Flemish bond",
    "hollow_brick": "Hollow brick",
    "ashlar": "Ashlar",
    "rammed_earth": "Rammed earth",
    "lean_concrete": "Lean concrete",
    "clay": "Clay",
    "rock": "Rock",
    "waterproofing": "Waterproofing membrane",
    "plaster": "Plaster",
    "acoustic": "Acoustic panel",
    "osb": "Oriented strand board",
    "clt": "Cross-laminated timber",
    "mesh": "Expanded metal",
    "chequer_plate": "Chequer plate",
    "shrubs": "Shrubs",
    "canopy": "Tree canopy",
    "crops": "Crops",
    "wetland": "Wetland",
    "herringbone": "Herringbone paving",
    "cobblestone": "Cobblestones",
    "asphalt": "Asphalt",
    "tactile": "Tactile paving",
    "wood_grain": "Wood, natural grain",
    "wood_herringbone": "Wood, herringbone",
    "wood_cross": "Timber cut across (cross)",
    "wood_open": "Wood, open grain",
    "wood_flame": "Wood, flame grain",
    "wood_diagonal": "Wood, diagonal grain",
    "wood_boards": "Wood, boards",
}

#: What each pattern is for, for the catalogue (docs/hatches.md) and the
#: pattern file it exports to: ``(alias, density, scale, angle, use)`` — a
#: short code, how dense it reads (``low`` / ``medium`` / ``high``), the
#: scale and the angle that suit it best, and its usual use.
HATCH_INFO = {
    "solid": ("SOLID", "high", 1.0, 0, "Thin cut elements, steel at small scale, poché"),
    "diagonal": ("DIAG", "medium", 1.0, 0, "Any material, the generic section hatch"),
    "dashed_diagonal": ("DIAG-DASH", "low", 1.0, 0, "To demolish, provisional or projected parts"),
    "double_diagonal": ("DIAG-2", "medium", 1.0, 0, "A second generic material beside the first"),
    "crosshatch": ("CROSS", "high", 1.0, 0, "Generic dense material, details"),
    "grid": ("GRID", "medium", 1.0, 0, "Grid floors, square tiling, reference areas"),
    "dots": ("DOTS", "low", 1.0, 0, "Light fills, zones, soft materials"),
    "plus": ("PLUS", "low", 1.0, 0, "Zoning: areas to keep, protected or reserved"),
    "hexagon": ("HEX", "low", 1.0, 0, "Hexagonal tiles, honeycomb panels, zoning"),
    "brick": ("BRICK", "medium", 1.0, 0, "Brick walls, stretcher bond"),
    "brick_bond": ("BRICK-EN", "medium", 1.0, 0, "Brick walls, English bond (one brick thick)"),
    "brick_flemish": ("BRICK-FL", "medium", 1.0, 0, "Brick walls and facings, Flemish (Gothic) bond, one brick thick"),
    "brick_auto": ("BRICK-AUTO", "medium", 1.0, 0, "Brick walls of any thickness: the bond follows each wall (partition, half brick, one, one and a half)"),
    "brick_header": ("BRICK-HD", "medium", 1.0, 0, "Load-bearing brick walls one brick thick, headers on the face"),
    "brick_cross": ("BRICK-X", "medium", 1.0, 0, "Load-bearing walls, English cross bond: the stretcher courses shift half a brick"),
    "brick_dutch": ("BRICK-NL", "medium", 1.0, 0, "Facings, Dutch bond: Flemish courses between header courses"),
    "brick_one_half": ("BRICK-1H", "medium", 1.0, 0, "Brick walls one and a half bricks thick"),
    "brick_on_edge": ("BRICK-EDGE", "low", 1.0, 0, "Partitions of brick on edge, carrying only themselves"),
    "brick_rowlock": ("BRICK-ROW", "medium", 1.0, 0, "Rowlocks: steps, sills, copings, lintels"),
    "brick_honeycomb": ("BRICK-HC", "low", 1.0, 0, "Honeycomb walls with gaps for air: screens, vents"),
    "hollow_brick": ("BRICK-HOL", "medium", 1.0, 0, "Hollow or perforated brick partitions"),
    "fire_brick": ("FIREBRICK", "medium", 1.0, 0, "Refractory linings: chimneys, ovens"),
    "block": ("BLOCK", "medium", 1.0, 0, "Concrete block walls"),
    "aac_block": ("AAC", "low", 1.0, 0, "Aerated (cellular) concrete blocks: large, light, laid with a thin glued joint"),
    "perforated_block": ("CERAMIC-LB", "medium", 1.0, 0, "Load-bearing ceramic blocks and perforated bricks (rejillón): holes running upright"),
    "concrete_brick": ("CONC-BRICK", "medium", 1.0, 0, "Precast concrete bricks with a few large holes running upright"),
    "ashlar": ("ASHLAR", "low", 1.0, 0, "Dressed stone walls in regular courses"),
    "stone": ("STONE", "medium", 1.0, 0, "Stone walls, cladding"),
    "rubble": ("RUBBLE", "medium", 1.0, 0, "Rubble masonry, old walls, gabions"),
    "rammed_earth": ("RAMMED", "low", 1.0, 0, "Rammed earth and adobe walls"),
    "tile": ("TILE", "medium", 1.0, 0, "Ceramic tiling, wall and floor finishes"),
    "concrete": ("CONC", "medium", 1.0, 0, "Plain concrete"),
    "reinforced": ("CONC-R", "medium", 1.0, 0, "Reinforced concrete: slabs, beams, columns"),
    "lean_concrete": ("CONC-LEAN", "low", 1.0, 0, "Blinding and lean concrete under foundations"),
    "sand": ("SAND", "high", 1.0, 0, "Sand, mortar beds, screeds"),
    "gravel": ("GRAVEL", "medium", 1.0, 0, "Gravel, hardcore, drainage layers"),
    "earth": ("EARTH", "medium", 1.0, 0, "Natural ground in section"),
    "fill": ("FILL", "medium", 1.0, 0, "Made ground, compacted fill"),
    "clay": ("CLAY", "medium", 1.0, 0, "Clay and silt layers in ground sections"),
    "rock": ("ROCK", "low", 1.0, 0, "Bedrock and rock in ground sections"),
    "insulation": ("INSUL", "medium", 1.0, 0, "Soft insulation: mineral wool, batts — fitted to the layer's thickness"),
    "insulation_zigzag": ("INSUL-ZZ", "low", 1.0, 0, "Insulation in thin layers: boards, panels, cavities — one zigzag from face to face"),
    "rigid_insulation": ("INSUL-R", "medium", 1.0, 0, "Rigid insulation boards: EPS, XPS, PIR"),
    "waterproofing": ("WATERPROOF", "high", 1.0, 0, "Waterproofing and vapour membranes"),
    "plaster": ("PLASTER", "low", 1.0, 0, "Plaster, render, gypsum board"),
    "acoustic": ("ACOUSTIC", "low", 1.0, 0, "Acoustic and perforated panels"),
    "roof_tile": ("ROOF-TILE", "medium", 1.0, 0, "Roof tiles in section or elevation"),
    "wood": ("WOOD", "low", 1.0, 0, "Timber along the grain"),
    "wood_end": ("WOOD-END", "low", 1.0, 0, "Timber cut across the grain"),
    "plywood": ("PLY", "medium", 1.0, 0, "Plywood and laminated boards"),
    "osb": ("OSB", "medium", 1.0, 0, "Oriented strand board, chipboard"),
    "clt": ("CLT", "medium", 1.0, 0, "Cross-laminated timber panels"),
    "parquet": ("PARQUET", "medium", 1.0, 0, "Wooden floors, parquet in plan"),
    "steel": ("STEEL", "high", 1.0, 0, "Steel sections"),
    "aluminium": ("ALU", "medium", 1.0, 0, "Aluminium sections, frames"),
    "bronze": ("BRONZE", "medium", 1.0, 0, "Bronze, brass, copper"),
    "lead": ("LEAD", "high", 1.0, 0, "Lead, zinc, flashings"),
    "mesh": ("MESH", "medium", 1.0, 0, "Expanded metal, gratings, meshes"),
    "chequer_plate": ("CHEQUER", "medium", 1.0, 0, "Chequer plate floors, stair treads"),
    "glass": ("GLASS", "low", 1.0, 0, "Glass in section"),
    "plastic": ("PLASTIC", "medium", 1.0, 0, "Plastics, rubber, seals"),
    "grass": ("GRASS", "low", 1.0, 0, "Lawn, grass areas"),
    "shrubs": ("SHRUB", "low", 1.0, 0, "Shrub and hedge planting"),
    "canopy": ("TREES", "low", 1.0, 0, "Tree groups, woodland in plan"),
    "crops": ("CROPS", "low", 1.0, 0, "Crops, planted beds, rows"),
    "wetland": ("WETLAND", "low", 1.0, 0, "Marsh, wet meadow, rain gardens"),
    "water": ("WATER", "low", 1.0, 0, "Water: ponds, pools, rivers"),
    "herringbone": ("HERRING", "medium", 1.0, 0, "Herringbone paving, brick or wood"),
    "cobblestone": ("COBBLE", "medium", 1.0, 0, "Cobbled and setts paving"),
    "asphalt": ("ASPHALT", "high", 1.0, 0, "Asphalt, roads, parking"),
    "tactile": ("TACTILE", "low", 1.0, 0, "Tactile warning paving"),
    "wood_grain": ("WOOD-GRAIN", "low", 1.0, 0, "Natural timber seen along the grain, with its knots: boards, beams, cladding"),
    "wood_herringbone": ("WOOD-HERRING", "medium", 1.0, 0, "Herringbone wooden floors: long narrow boards"),
    "wood_cross": ("WOOD-X", "low", 1.0, 0, "Timber cut across its grain: studs, joists, beams, battens — a cross from corner to corner of each piece"),
    "wood_open": ("WOOD-OPEN", "low", 1.0, 0, "Timber in elevation, light: a few long grain lines"),
    "wood_flame": ("WOOD-FLAME", "high", 1.0, 0, "Flat-sawn boards: flame grain round the knots, veneers"),
    "wood_diagonal": ("WOOD-DIAG", "medium", 1.0, 0, "Timber with the grain running across: cladding, battens"),
    "wood_boards": ("WOOD-BOARDS", "medium", 1.0, 0, "Boarded floors and decks, timber cut lengthwise"),
}


def _basket() -> list:
    """Parquet in a basket weave: 3 mm squares, each of three strips, the
    strips turning a quarter from one square to the next."""
    fams = [(0.0, 0.0, 0.0, 0.0, 3.0, ()), (90.0, 0.0, 0.0, 0.0, 3.0, ())]
    for y in (1.0, 2.0):
        fams.append((0.0, 0.0, y, 0.0, 6.0, (3.0, -3.0)))
        fams.append((0.0, 3.0, 3.0 + y, 0.0, 6.0, (3.0, -3.0)))
    for x in (1.0, 2.0):
        fams.append((90.0, 3.0 + x, 0.0, 0.0, 6.0, (3.0, -3.0)))
        fams.append((90.0, x, 3.0, 0.0, 6.0, (3.0, -3.0)))
    return fams


#: Line families: ``(angle°, x0, y0, shift along, spacing across, dashes[,
#: offset across])`` in paper mm at scale 1. Dashes: positive draws,
#: negative is a gap, 0 a dot; empty: a whole line. The offset moves the
#: family across itself — the second line of a pair.
#: The brick patterns are not here: :func:`brick_families` draws them from
#: the brick's size.
_FAMILIES = {
    "diagonal": [(45.0, 0.0, 0.0, 0.0, 1.5, ())],
    "double_diagonal": [(45.0, 0.0, 0.0, 0.0, 2.0, ()),
                        (45.0, 0.0, 0.0, 0.0, 2.0, (), 0.35)],
    "crosshatch": [(45.0, 0.0, 0.0, 0.0, 1.5, ()),
                   (135.0, 0.0, 0.0, 0.0, 1.5, ())],
    "grid": [(0.0, 0.0, 0.0, 0.0, 1.5, ()), (90.0, 0.0, 0.0, 0.0, 1.5, ())],
    "dots": [(0.0, 0.0, 0.0, 0.5, 1.0, (0.0, -1.0))],
    "fire_brick": [(45.0, 0.0, 0.0, 0.0, 1.5, ()),
                   (135.0, 0.0, 0.0, 0.0, 3.0, (1.5, -1.5))],
    # Blocks 4 × 2 mm, half a block on, with a hollow line through each.
    # Aerated concrete: big blocks, half bond, the pores scattered.
    "aac_block": [(0.0, 0.0, 0.0, 0.0, 2.0, ()),
                  (90.0, 0.0, 0.0, 2.0, 3.0, (2.0, -2.0))],
    "block": [(0.0, 0.0, 0.0, 0.0, 2.0, ()),
              (90.0, 0.0, 0.0, 2.0, 4.0, (2.0, -2.0)),
              (0.0, 1.0, 1.0, 4.0, 2.0, (2.0, -6.0))],
    # Square tiles with a joint of two lines.
    "tile": [(0.0, 0.0, 0.0, 0.0, 3.0, ()), (0.0, 0.0, 0.0, 0.0, 3.0, (), 0.25),
             (90.0, 0.0, 0.0, 0.0, 3.0, ()),
             (90.0, 0.0, 0.0, 0.0, 3.0, (), 0.25)],
    "reinforced": [(45.0, 0.0, 0.0, 0.0, 2.4, ())],
    # Groups of three lines, then a gap — the classic earth section.
    "earth": [(45.0, 0.0, 0.0, 1.0, 3.0, (2.0, -1.0)),
              (45.0, 0.0, 0.0, 1.0, 3.0, (2.0, -1.0), 0.45),
              (45.0, 0.0, 0.0, 1.0, 3.0, (2.0, -1.0), 0.9)],
    # Made ground: broken lines the other way over a sand stipple.
    "fill": [(135.0, 0.0, 0.0, 0.7, 2.0, (1.0, -1.0))],
    # Boards of rigid insulation: rectangles crossed corner to corner.
    "rigid_insulation": [(0.0, 0.0, 0.0, 0.0, 2.0, ()),
                         (90.0, 0.0, 0.0, 0.0, 4.0, ()),
                         (math.degrees(math.atan2(1.0, 2.0)), 0.0, 0.0, 0.0,
                          2.0 * math.cos(math.atan2(1.0, 2.0)), ()),
                         (180.0 - math.degrees(math.atan2(1.0, 2.0)), 0.0,
                          0.0, 0.0, 2.0 * math.cos(math.atan2(1.0, 2.0)), ())],
    # Still water: short dashes in staggered rows.
    "water": [(0.0, 0.0, 0.0, 1.8, 1.4, (2.0, -1.6)),
              (0.0, 2.2, 0.0, 1.8, 1.4, (0.8, -2.8), 0.45)],
    # Plies and their grain.
    "plywood": [(0.0, 0.0, 0.0, 0.0, 1.0, ()),
                (0.0, 0.0, 0.0, 1.7, 2.0, (2.5, -1.0), 0.5)],
    "parquet": _basket(),
    "dashed_diagonal": [(45.0, 0.0, 0.0, 0.75, 1.5, (1.0, -0.5))],
    # A cross at every node of a 3.5 mm grid.
    "plus": [(0.0, -0.8, 0.0, 0.0, 3.5, (1.6, -1.9)),
             (90.0, 0.0, -0.8, 0.0, 3.5, (1.6, -1.9))],
    # Honeycomb of 1.2 mm sides: three families, a side drawn, two skipped.
    "hexagon": [(0.0, 0.0, 0.0, 1.8, 1.2 * math.sqrt(3) / 2, (1.2, -2.4)),
                (120.0, 0.0, 0.0, 1.8, 1.2 * math.sqrt(3) / 2, (1.2, -2.4)),
                (60.0, 1.2, 0.0, 1.8, 1.2 * math.sqrt(3) / 2, (1.2, -2.4))],
    # Dressed stone: courses 3 mm high, blocks 8 mm long.
    "ashlar": [(0.0, 0.0, 0.0, 0.0, 3.0, ()),
               (90.0, 0.0, 0.0, 3.0, 4.0, (3.0, -3.0))],
    # Lifts of rammed earth over a light stipple.
    "rammed_earth": [(0.0, 0.0, 0.0, 0.0, 2.5, ())],
    # Clay: broken horizontal lines with a dot between.
    "clay": [(0.0, 0.0, 0.0, 1.3, 1.2, (2.0, -0.6)),
             (0.0, 2.3, 0.6, 1.3, 1.2, (0.0, -2.6))],
    # A membrane: a dense dash-dot line.
    "waterproofing": [(0.0, 0.0, 0.0, 0.8, 0.7, (1.5, -0.4, 0.0, -0.4))],
    # Cross-laminated timber: plies of grain along and across in turn.
    "clt": [(0.0, 0.0, 0.0, 0.0, 2.0, ()),
            (90.0, 0.0, 0.0, 0.0, 0.6, (2.0, -2.0)),
            (0.0, 0.0, 3.0, 0.8, 4.0, (1.6, -0.6))],
    # Diamonds of expanded metal.
    "mesh": [(60.0, 0.0, 0.0, 0.0, 1.4, ()), (120.0, 0.0, 0.0, 0.0, 1.4, ())],
    # Rows of ticks: crops, planted beds.
    "crops": [(90.0, 0.0, 0.0, 0.0, 1.6, (0.8, -1.4))],
    # Wetland: water's dashes; tufts are scattered over them.
    "wetland": [(0.0, 0.0, 0.0, 1.8, 1.4, (2.0, -1.6))],
    # Thin lines in close pairs.
    "steel": [(45.0, 0.0, 0.0, 0.0, 1.2, ()),
              (45.0, 0.0, 0.0, 0.0, 1.2, (), 0.3)],
    # A line and a dashed line beside it.
    "aluminium": [(45.0, 0.0, 0.0, 0.0, 2.0, ()),
                  (45.0, 0.0, 0.0, 0.0, 2.0, (1.0, -0.6), 0.5)],
    # A line and a dash-dot line between.
    "bronze": [(45.0, 0.0, 0.0, 0.0, 2.0, ()),
               (45.0, 0.0, 0.0, 0.0, 2.0, (2.0, -0.5, 0.0, -0.5), 1.0)],
    # Crossed pairs.
    "lead": [(45.0, 0.0, 0.0, 0.0, 2.0, ()),
             (45.0, 0.0, 0.0, 0.0, 2.0, (), 0.35),
             (135.0, 0.0, 0.0, 0.0, 2.0, ()),
             (135.0, 0.0, 0.0, 0.0, 2.0, (), 0.35)],
    # Long and short dashes, in pairs.
    "glass": [(45.0, 0.0, 0.0, 0.0, 2.4, (4.0, -1.5)),
              (45.0, 0.0, 0.0, 2.0, 2.4, (1.5, -4.0), 0.5)],
    # Pairs of lines with a broken one between them.
    "plastic": [(45.0, 0.0, 0.0, 0.0, 3.0, ()),
                (45.0, 0.0, 0.0, 0.0, 3.0, (), 0.5),
                (45.0, 0.0, 0.0, 0.0, 3.0, (0.8, -0.8), 1.6)],
}

#: Generated patterns: ``(kind, cell size in mm)``.
_SCATTERED = {"aac_block": ("pores", 1.1), "concrete": ("concrete", 2.0), "reinforced": ("concrete", 2.0),
              "gravel": ("gravel", 1.6), "sand": ("sand", 0.9),
              "fill": ("sand", 1.3), "stone": ("stone", 3.0),
              "rubble": ("rubble", 2.6), "roof_tile": ("scales", 3.0),
              "grass": ("grass", 2.4), "wood_end": ("rings", 5.0),
              "rammed_earth": ("plaster", 1.8), "lean_concrete": ("concrete", 3.4),
              "rock": ("rock", 3.5), "plaster": ("plaster", 1.2),
              "acoustic": ("studs", 1.5), "tactile": ("studs", 2.5),
              "osb": ("strands", 1.6), "chequer_plate": ("chequer", 2.0),
              "shrubs": ("shrubs", 3.0), "canopy": ("canopy", 5.0),
              "wetland": ("grass", 3.2), "cobblestone": ("cobbles", 2.4),
              "asphalt": ("asphalt", 0.7),
              "wood_grain": ("grain", 1.0), "wood_herringbone": ("herring5", 0.9),
              "herringbone": ("herring2", 1.6),
              "wood_open": ("grain_open", 2.0), "wood_flame": ("grain_flame", 0.85),
              "wood_boards": ("ticks", 1.5)}
#: The materials a pattern draws at their real size: ``(unit, metres)`` —
#: the pattern's own measure on paper (a brick's course, a tile, a board's
#: width, in mm at scale 1) and what it stands for in reality. Such a
#: pattern grows and shrinks with the frame's scale, unless the layer says
#: otherwise; stipples and generic hatches keep their size on paper, the
#: drawing convention (shrunk to real size they would not read).
REAL_SIZE = {
    "block": (2.0, 0.20), "aac_block": (2.0, 0.203),
    "ashlar": (3.0, 0.40), "tile": (3.0, 0.30),
    "roof_tile": (3.0, 0.25), "parquet": (3.0, 0.24),
    "herringbone": (1.6, 0.10), "wood_herringbone": (0.9, 0.07),
    "wood_boards": (1.5, 0.14), "cobblestone": (2.4, 0.12),
    "tactile": (2.5, 0.06),
}
#: The brickwork patterns: drawn from the size of the brick (a layer may
#: set its own, ``Layer.brick``) and from the bond — how stretchers (the
#: long side on the face) and headers (the short end on the face) are laid
#: course by course.
BRICKS = ("brick_auto", "brick", "brick_header", "brick_bond", "brick_cross",
          "brick_flemish", "brick_dutch", "brick_one_half", "brick_on_edge",
          "brick_rowlock", "brick_honeycomb", "hollow_brick",
          "perforated_block", "concrete_brick")
#: A brick: ``(length, width, height, joint)`` in mm. The one the patterns
#: are drawn with unless a layer says otherwise, and the usual sizes of a
#: few places (``(name, size)``), offered by the picker.
BRICK_DEFAULT = (215.0, 102.5, 65.0, 10.0)
BRICK_SIZES = (("Metric", BRICK_DEFAULT),
               ("Spain", (240.0, 115.0, 50.0, 10.0)),
               ("Chile", (290.0, 140.0, 71.0, 10.0)),
               ("Argentina", (260.0, 125.0, 55.0, 10.0)),
               ("Mexico", (280.0, 140.0, 70.0, 10.0)),
               ("Uruguay", (250.0, 120.0, 55.0, 10.0)))
#: The masonry pieces a layer sizes, by pattern: a brick, a hollow
#: ceramic block (a «ticholo») and a concrete block — each with its usual
#: sizes ``(name, (length, width, height, joint))`` in mm, the first the
#: one drawn unless the layer or the document says otherwise. (Dressed
#: stone keeps its own: no two stones alike.)
PIECE_SIZES = {
    "brick": BRICK_SIZES,
    "hollow": (("Ticholo 12", (250.0, 120.0, 170.0, 10.0)),
               ("Ticholo 8", (250.0, 80.0, 170.0, 10.0)),
               ("Hollow brick 12", (330.0, 120.0, 180.0, 10.0)),
               ("Hollow brick 18", (330.0, 180.0, 180.0, 10.0))),
    "perforated": (("Load-bearing 12", (250.0, 120.0, 190.0, 10.0)),
                   ("Load-bearing 18", (330.0, 180.0, 190.0, 10.0)),
                   ("Rejillón", (250.0, 120.0, 100.0, 10.0))),
    "cored": (("Concrete brick 12", (250.0, 120.0, 60.0, 10.0)),
              ("Concrete brick 14", (290.0, 140.0, 90.0, 10.0))),
    "block": (("Block 15", (390.0, 140.0, 190.0, 10.0)),
              ("Block 10", (390.0, 90.0, 190.0, 10.0)),
              ("Block 20", (390.0, 190.0, 190.0, 10.0))),
    # Glued: a joint of a few millimetres.
    "aac": (("Aerated 15", (600.0, 150.0, 200.0, 3.0)),
            ("Aerated 10", (600.0, 100.0, 200.0, 3.0)),
            ("Aerated 20", (600.0, 200.0, 200.0, 3.0)),
            ("Aerated 25", (600.0, 250.0, 200.0, 3.0))),
}
PIECE_DEFAULT = {kind: sizes[0][1] for kind, sizes in PIECE_SIZES.items()}


def piece_kind(key) -> str | None:
    """Which piece pattern ``key`` is made of — ``"brick"``, ``"hollow"``
    (a ticholo), ``"perforated"`` (a load-bearing ceramic block, a
    rejillón), ``"cored"`` (a precast concrete brick with holes),
    ``"block"``, ``"aac"`` (aerated concrete) — or ``None`` for
    one with no size to set."""
    if key == "block":
        return "block"
    if key == "aac_block":
        return "aac"
    if key == "hollow_brick":
        return "hollow"
    if key == "perforated_block":
        return "perforated"
    if key == "concrete_brick":
        return "cored"
    return "brick" if key in BRICKS else None


#: What a brick size may be, in mm: ``(least, most)`` of its length, width,
#: height and joint.
_BRICK_LIMITS = ((60.0, 600.0), (30.0, 400.0), (20.0, 400.0), (0.0, 40.0))
for _key in BRICKS:
    REAL_SIZE[_key] = (1.5, (BRICK_DEFAULT[2] + BRICK_DEFAULT[3]) / 1000.0)

#: Masonry laid in courses: drawn as it is built — in a plan, the top of a
#: course, brick by brick along each wall; in a vertical section through a
#: wall, the courses stacked and each split where its bricks meet; cut along
#: a wall, its face (see :func:`bond_marks`).
COURSED = frozenset(BRICKS) | {"block", "aac_block", "ashlar"}
#: The patterns made of pieces — bricks, blocks, stones, tiles, boards,
#: pavers — where it shows where the first joint falls, so a layer can
#: shift them (``Layer.hatch_offset``). Lines and textures read the same
#: wherever they start.
PLACED = COURSED | {"fire_brick", "stone", "tile", "roof_tile", "wood_boards",
                    "parquet", "wood_herringbone", "clt", "herringbone",
                    "cobblestone", "tactile", "chequer_plate", "grid"}
#: The thickest piece, in real mm, still drawn brick by brick: in a plan, a
#: wall (thicker, a paved area); in a vertical section, a wall cut across
#: (wider, one cut along: its face).
_BOND_THICKEST = {"plan": 1500.0, "vertical": 700.0}
#: Bonds whose face is another's: they part in a plan or a section (by
#: thickness: the bond each wall takes; one and a half: a header on one
#: face, a stretcher on the other).
SAME_FACE = {"brick_auto": "brick", "brick_one_half": "brick_bond"}
#: Dressed stone: ``(length, course height)`` in mm.
_UNITS = {"ashlar": (800.0, 400.0)}

#: However large or small the scale, a real-size pattern is drawn between
#: these many times its own size: smaller is a grey blur, larger a few
#: lines lost in the cut.
_REAL_LIMITS = (0.25, 12.0)


def clean_brick(raw) -> tuple | None:
    """A brick size as a layer stores it, read back defensively: ``(length,
    width, height, joint)`` in mm within reason, the width no more than the
    length — or ``(length, width, height, bed joint, head joint)`` when the
    horizontal joint (the bed, under each course) and the vertical one (the
    head, between two bricks) differ; ``None`` for anything else. Equal
    joints come back as one."""
    try:
        size = tuple(float(v) for v in raw)
    except (TypeError, ValueError):
        return None
    if len(size) not in (4, 5) or not all(math.isfinite(v) for v in size):
        return None
    limits = _BRICK_LIMITS + (_BRICK_LIMITS[3],)
    if any(not lo <= v <= hi for v, (lo, hi) in zip(size, limits)):
        return None
    if size[1] > size[0]:
        return None
    if len(size) == 5 and abs(size[3] - size[4]) < 1e-9:
        size = size[:4]
    return size


def brick_dims(brick=None, kind: str = "brick") -> tuple:
    """``(length, width, height, bed joint, head joint)`` of ``brick`` (a
    layer's, the document's) in mm — the usual piece of ``kind`` when it
    has none: the bed joint lies under each course, the head joint stands
    between two pieces of a course and between the wythes of a wall."""
    size = clean_brick(brick) or PIECE_DEFAULT.get(kind, BRICK_DEFAULT)
    return size if len(size) == 5 else size + (size[3],)


def real_unit(key, brick=None) -> tuple | None:
    """``(unit, metres)`` of a real-size pattern — REAL_SIZE's, a brick
    pattern's from ``brick`` (its course: height and joint)."""
    if key in BRICKS:
        b = brick_dims(brick, piece_kind(key))
        return 1.5, (b[2] + b[3]) / 1000.0     # height and bed joint
    if key in ("block", "aac_block"):
        b = brick_dims(brick, piece_kind(key))
        return REAL_SIZE[key][0], (b[2] + b[3]) / 1000.0
    return REAL_SIZE.get(key)


def real_scale(key, scale_n: float, brick=None) -> float | None:
    """How many times its own size pattern ``key`` is drawn on a sheet at
    1:``scale_n`` to show the material at its real size (kept within
    readable limits); ``None`` for a pattern with no real size. A brick
    pattern measures its course with ``brick`` (``Layer.brick``)."""
    if key not in REAL_SIZE:
        return None
    unit, metres = real_unit(key, brick)
    factor = metres * 1000.0 / max(1e-6, float(scale_n or 100.0)) / unit
    return min(_REAL_LIMITS[1], max(_REAL_LIMITS[0], factor))


#: Patterns drawn turned from their families (degrees).
_TURN = {"herringbone": 45.0, "wood_herringbone": 45.0,
         "wood_flame": 25.0, "wood_diagonal": 35.0}

#: Waves: ``(band height, period, amplitude)`` in mm.
_WAVES = {"wood": (1.0, 7.0, 0.35), "wood_diagonal": (1.7, 14.0, 0.35)}
#: Patterns fitted to each cut piece instead of laid on the paper: the
#: soft insulation symbol spans the layer's thickness, whatever it is and
#: whichever way the layer runs. Laid on the paper (swatches, legends), it
#: comes in bands of this thickness, in mm.
FITTED = {"insulation": 3.0, "insulation_zigzag": 3.0, "wood_cross": 3.0}
#: The insulation symbol: the radius of its half loops, as a share of the
#: layer's thickness — arches against one face and cups against the other,
#: each touching the next on its face.
_BATT_RADIUS = 0.22


#: The key prefix of a pattern brought in from a pattern file (``.pat``) and
#: kept in the document (``Scene.hatch_patterns``): ``"pat:<name>"``.
IMPORTED = "pat:"
#: A pattern file's limits: what a reasonable pattern needs, and no more —
#: a file is data from anywhere.
_MAX_FAMILIES = 64
_MAX_DASHES = 32
#: The fewest millimetres between two lines of a family: closer, and a big
#: cut would be inked with hundreds of thousands of lines.
_MIN_SPACING = 0.05


def brick_families(key, brick=None) -> list:
    """The families of brick pattern ``key`` seen on the face of the wall,
    from ``brick`` (``(length, width, height, joint)`` mm; the default one
    when ``None``): the course 1.5 mm high, the rest in proportion — a
    stretcher as long as the brick, a header as wide."""
    length, width, height, bed_joint, head = brick_dims(brick,
                                                       piece_kind(key))
    u = 1.5 / (height + bed_joint)
    ls, ws, hs = (length + head) * u, (width + head) * u, 1.5
    bed = [(0.0, 0.0, 0.0, 0.0, hs, ())]

    def stretchers(h):
        # Every course half a brick on from the one below.
        return [(0.0, 0.0, 0.0, 0.0, h, ()),
                (90.0, 0.0, 0.0, h, ls / 2.0, (h, -h))]
    if key == "brick_on_edge":         # on edge: the course as high as wide
        return stretchers((width + bed_joint) * u)
    if key == "brick_rowlock":         # standing ends, side by side
        return [(0.0, 0.0, 0.0, 0.0, (width + bed_joint) * u, ()),
                (90.0, 0.0, 0.0, 0.0, (height + head) * u, ())]
    if key == "brick_header":
        return bed + [(90.0, 0.0, 0.0, hs, ws / 2.0, (hs, -hs))]
    if key in ("brick_bond", "brick_one_half"):
        # A course of stretchers, a course of headers over their joints.
        return bed + [(90.0, 0.0, 0.0, 0.0, ls, (hs, -hs)),
                      (90.0, ws / 2.0, hs, 0.0, ws, (hs, -hs))]
    if key == "brick_cross":
        # English, every second course of stretchers half a brick on.
        return bed + [(90.0, 0.0, 0.0, 0.0, ls, (hs, -3.0 * hs)),
                      (90.0, ls / 2.0, 2.0 * hs, 0.0, ls, (hs, -3.0 * hs)),
                      (90.0, ws / 2.0, hs, 0.0, ws, (hs, -hs))]
    if key in ("brick_flemish", "brick_dutch"):
        # A stretcher and a header in turn along the course; Flemish: the
        # header centred on the stretcher below; Dutch: a course of
        # headers between.
        p = ls + ws
        fams = bed + [(90.0, 0.0, 0.0, 0.0, p, (hs, -hs)),
                      (90.0, ls, 0.0, 0.0, p, (hs, -hs))]
        if key == "brick_dutch":
            return fams + [(90.0, ws / 2.0, hs, 0.0, ws, (hs, -hs))]
        return fams + [(90.0, ls / 2.0 - ws / 2.0, hs, 0.0, p, (hs, -hs)),
                       (90.0, ls / 2.0 + ws / 2.0, hs, 0.0, p, (hs, -hs))]
    if key == "brick_honeycomb":
        # Stretchers with a gap of half a brick between, staggered.
        gap = ls / 2.0
        half = (ls + gap) / 2.0
        return bed + [(90.0, 0.0, 0.0, hs, half, (hs, -hs)),
                      (90.0, gap, 0.0, hs, half, (hs, -hs))]
    fams = stretchers(hs)              # stretcher bond, by thickness, hollow
    if key == "hollow_brick":
        k = ls / 6.0                   # the holes as short dashes
        fams.append((0.0, 0.6 * k, hs / 2.0, ls / 2.0, hs,
                     tuple(d * k for d in (0.9, -0.4, 0.9, -0.4, 0.9, -2.5))))
    return fams


for _key in BRICKS:
    _FAMILIES[_key] = brick_families(_key)


def vertical_band(ring) -> tuple | None:
    """A piece of a vertical section as :func:`fit_band` gives one, but
    upright whatever its shape: along the paper's up (courses are
    horizontal), across its width."""
    xs = [float(p[0]) for p in ring]
    ys = [float(p[1]) for p in ring]
    if len(xs) < 3:
        return None
    w, h = max(xs) - min(xs), max(ys) - min(ys)
    if w <= 1e-9 or h <= 1e-9:
        return None
    return (min(xs), max(ys)), (0.0, -1.0), (1.0, 0.0), h, w


def bond_course(key, k: int, thick: float, brick=None, lo: float = 0.0,
                hi: float = 0.0) -> tuple:
    """Course ``k`` of masonry pattern ``key`` in a wall ``thick`` mm thick,
    as built: ``(depth, height, pieces)`` — the course's depth across the
    wall (its pieces fill it), its height, and its pieces between ``lo`` and
    ``hi`` along the wall: ``(a0, a1, b0, b1, kind)`` in mm, ``kind`` one
    of ``"brick"``, ``"hollow"``, ``"block"``, ``"gap"`` (an open one)."""
    return masonry_course(key, k, thick, brick_dims(brick, piece_kind(key)), lo, hi)


#: How masonry joints are drawn: as one line, as two (the joint's real
#: thickness), not at all (the outline only); ``None`` — by the sheet's
#: scale (:func:`joints_mode`).
JOINTS = ("single", "double", "none")
#: The automatic joints: a 45° hatch at 1:``[1]`` and smaller (a brick is
#: a couple of millimetres: drawn, it is a grey blur), single lines down to
#: 1:``[0]``, the joint's real thickness from there on (1 cm: 1 mm at 1:10).
JOINTS_AUTO = (15.0, 75.0)


def joints_mode(joints, scale_n: float = 100.0) -> str:
    """How a sheet at 1:``scale_n`` draws masonry joints: ``"single"``,
    ``"double"``, ``"none"`` or ``"hatch"`` (no bricks: the 45° hatch) — as
    the layer says (``Layer.joints``), else by the scale."""
    if joints in JOINTS:
        return joints
    n = float(scale_n or 100.0)
    if n >= JOINTS_AUTO[1]:
        return "hatch"
    return "double" if n <= JOINTS_AUTO[0] else "single"


def bond_marks(key, band, cut: str, scale: float = 1.0,
               scale_n: float = 100.0, brick=None,
               joints: str = "single", offset=None) -> dict | None:
    """Masonry drawn as it is built in one cut piece ``band`` (``(origin,
    along, across, length, thick)`` in paper mm, as :func:`fit_band` or
    :func:`vertical_band` give it): in a ``"plan"``, the top of a course,
    brick by brick along the wall; in a ``"vertical"`` section, the courses
    stacked along ``along`` (up), each split across where its bricks meet.
    The pieces fill the thickness; along the wall they measure their size
    ``scale`` times (as the pattern's: real at 1:``scale_n``, or on paper),
    the bond chosen from the wall's real thickness. A piece too thick for a
    wall (a paved area, a wall cut along) shows the bond's face, brick by
    brick. ``joints``: ``"single"`` — a line at every joint; ``"double"`` —
    each brick its own outline, the joint's real thickness between.
    ``{"lines", "polys", "bricks"}`` in paper mm (``bricks``: the closed
    outlines of the double joints, to fill and ink); ``None`` for no
    masonry or no piece.

    Each piece starts as a wall is built: a whole brick at its end (the
    left one, or the top one on the paper) in a plan, the first course at
    its foot in a section. ``offset`` — ``(x, y)`` in real mm — moves the
    first joint along the wall (``x``, a plan) and the first course up it
    (``y``, a section); a paved area or a wall's face moves both ways."""
    if key not in COURSED or band is None or cut not in _BOND_THICKEST:
        return None
    return masonry_marks(key, band, cut, scale, float(scale_n or 100.0),
                         brick_dims(brick, piece_kind(key)), real_unit(key, brick),
                         joints, offset)


def paper_offset(form) -> tuple | None:
    """A layer's shift of its pattern (``form["offset"]``, real mm, y up)
    on a sheet at 1:``form["scale_n"]``: paper mm, y down — or ``None``."""
    off = (form or {}).get("offset")
    if not off or not (off[0] or off[1]):
        return None
    n = max(1e-6, float((form or {}).get("scale_n", 100.0) or 100.0))
    return float(off[0]) / n, -float(off[1]) / n


def shift_marks(marks, dx: float, dy: float) -> dict:
    """``marks`` (as :func:`hatch_marks` gives them) moved by ``(dx,
    dy)``."""
    out = dict(marks)
    out["lines"] = [(x0 + dx, y0 + dy, x1 + dx, y1 + dy)
                    for x0, y0, x1, y1 in marks.get("lines", ())]
    out["dots"] = [(x + dx, y + dy) for x, y in marks.get("dots", ())]
    out["circles"] = [(x + dx, y + dy, r)
                      for x, y, r in marks.get("circles", ())]
    for key in ("polys", "closed", "bricks"):
        if key in marks:
            out[key] = [[(x + dx, y + dy) for x, y in pts]
                        for pts in marks[key]]
    return out


def _segments(marks) -> list:
    """Every mark as straight segments ``(x0, y0, x1, y1)``: lines as they
    are, polylines and outlines edge by edge, circles as 16-sided rounds;
    dots, which a line drawing cannot hold, left out."""
    out = list(marks.get("lines", ()))
    for key in ("polys", "closed", "bricks"):
        for pts in marks.get(key, ()):
            pts = list(pts)
            if key == "closed" and len(pts) > 2:
                pts.append(pts[0])
            out += [(a[0], a[1], b[0], b[1]) for a, b in zip(pts, pts[1:])]
    for x, y, r in marks.get("circles", ()):
        ring = [(x + r * math.cos(2 * math.pi * i / 16),
                 y + r * math.sin(2 * math.pi * i / 16)) for i in range(17)]
        out += [(a[0], a[1], b[0], b[1]) for a, b in zip(ring, ring[1:])]
    return out


def clip_segments(lines, rings) -> list:
    """Clip an iterable of paper segments against the even-odd fill."""
    return hatch_clip_segments(list(lines), list(rings))


def fill_quads(rings) -> list:
    """Return ordered bottom and top corners for each DXF SOLID strip."""
    return hatch_fill_quads(list(rings))


def hatch_lines(key, rings, form=None, frame_scale: float = 1.0,
                depths=None) -> list:
    """Pattern ``key`` over the cut ``rings`` (paper mm, y down) as a
    sheet draws it — turned, sized, masonry as it is built, fitted
    patterns piece by piece, wood crossed where it is cut across — but as
    straight segments clipped to the rings: what a DXF can hold. ``form``
    as core.layers.cut_hatch_forms gives it (plus ``"cut"``); ``frame_scale``
    the frame's own hatch size (its spacing / 1.5); ``depths`` how far each
    ring's solid runs through the plane (paper mm, ``None`` unknown) — the
    grain of timber. Solid fills and the dots of stipples have no line form
    and stay out."""
    form = form or {}
    rings = [[(float(x), float(y)) for x, y in r] for r in rings
             if len(r) >= 3]
    if not rings or key in (None, "solid"):
        return []
    cut = form.get("cut")
    n = float(form.get("scale_n", 100.0) or 100.0)
    scale = frame_scale * float(form.get("scale", 1.0) or 1.0)
    out = []
    if key in WOOD and form.get("cross") and cut:
        rest = []
        depths = list(depths) if depths is not None else [None] * len(rings)
        for r, depth in zip(rings, depths):
            band = fit_band(r)
            if cut_across(band, n, depth):
                out += clip_segments(_segments(
                    {"polys": fitted_marks("wood_cross", *band)}), [r])
            else:
                rest.append(r)
        rings = rest
        if not rings:
            return out
    if key in COURSED and cut in ("plan", "vertical"):
        mode = joints_mode(form.get("joints"), n)
        if mode == "none":
            return out
        if mode == "hatch":
            key, scale = "diagonal", frame_scale * float(
                form.get("hatch_scale", 1.0) or 1.0)
        else:
            for r in rings:
                band = fit_band(r) if cut == "plan" else vertical_band(r)
                marks = bond_marks(key, band, cut, scale, n,
                                   form.get("brick"), mode,
                                   form.get("offset"))
                if marks is not None:
                    out += clip_segments(_segments(marks), [r])
            return out
    if key in FITTED:
        for r in rings:
            band = fit_band(r)
            if band is not None:
                out += clip_segments(_segments(
                    {"polys": fitted_marks(key, *band)}), [r])
        return out
    xs = [p[0] for r in rings for p in r]
    ys = [p[1] for r in rings for p in r]
    marks = hatch_marks(key, (min(xs), min(ys), max(xs), max(ys)), scale,
                        float(form.get("angle", 0.0) or 0.0),
                        form.get("families") if key not in COURSED
                        or key in BRICKS else None,
                        paper_offset(form) if key in PLACED else None)
    return out + clip_segments(_segments(marks), rings)


def known(key) -> bool:
    """Whether ``key`` names a pattern (``None`` — the frame's fill — too):
    one of the built-in ones, or an imported one (``pat:<name>``)."""
    return (key is None or key in HATCH_LABELS
            or (isinstance(key, str) and key.startswith(IMPORTED)
                and len(key) > len(IMPORTED)))


def parse_pat(text: str) -> dict:
    """The patterns of a pattern file (``.pat``) — the plain text format of
    technical drawing: ``*NAME, description`` and then one line per family,
    ``angle, x0, y0, shift along, spacing across[, dash, …]`` (positive
    draws, negative is a gap, 0 a dot). Returns ``{name: {"label": …,
    "families": [[angle, x0, y0, along, across, [dashes]], …]}}`` in
    millimetres: a file that says ``;%UNITS=INCH`` is turned from inches.
    Lines that cannot be read are skipped; a pattern with no family left is
    dropped."""
    factor = 25.4 if re.search(r";\s*%\s*UNITS\s*=\s*INCH", text,
                               re.IGNORECASE) else 1.0
    out: dict = {}
    current = None
    for raw in text.splitlines():
        line = raw.split(";", 1)[0].strip()
        if not line:
            continue
        if line.startswith("*"):
            name, _, label = line[1:].partition(",")
            name = name.strip()[:64]
            if not name:
                current = None
                continue
            current = out.setdefault(name, {"label": label.strip()[:120]
                                            or name, "families": []})
            continue
        if current is None or len(current["families"]) >= _MAX_FAMILIES:
            continue
        try:
            nums = [float(v) for v in line.split(",") if v.strip()]
        except ValueError:
            continue
        if len(nums) < 5 or not all(math.isfinite(v) for v in nums):
            continue
        angle, x0, y0, along, across = nums[:5]
        if abs(across * factor) < _MIN_SPACING:
            continue
        dashes = [d * factor for d in nums[5:5 + _MAX_DASHES]]
        current["families"].append([angle % 360.0, x0 * factor, y0 * factor,
                                    along * factor, across * factor, dashes])
    return {n: p for n, p in out.items() if p["families"]}


def line_families(key) -> list | None:
    """The families of a built-in pattern drawn only with lines, as a
    pattern file writes them (``[angle, x0, y0, along, across, [dashes]]``,
    the offset across folded into the origin, the built-in turn applied);
    ``None`` for one generated (stipples, stones, rounds, waves), which no
    pattern file can describe."""
    if key not in _FAMILIES or key in _SCATTERED or key in _WAVES:
        return None
    turn = _TURN.get(key, 0.0)
    t = math.radians(turn)
    out = []
    for fam in _FAMILIES[key]:
        angle, x0, y0, along, across, dashes = fam[:6]
        off = fam[6] if len(fam) > 6 else 0.0
        if off:
            a = math.radians(angle)
            x0 -= math.sin(a) * off
            y0 += math.cos(a) * off
        if turn:
            x0, y0 = (x0 * math.cos(t) - y0 * math.sin(t),
                      x0 * math.sin(t) + y0 * math.cos(t))
        out.append([(angle + turn) % 360.0, x0, y0, along, across,
                    list(dashes)])
    return out


def to_pat(patterns) -> str:
    """A pattern file (``.pat``, millimetres) of ``patterns`` — ``[(name,
    label, families)]`` — the plain text format :func:`parse_pat` reads and
    the drawing programs share."""
    lines = [";; Hatch patterns exported from IngeTrazo",
             ";%UNITS=MM", ""]

    def num(v):
        text = f"{v:.6f}".rstrip("0").rstrip(".")
        return "0" if text in ("-0", "") else text
    for name, label, families in patterns:
        if not families:
            continue
        safe = re.sub(r"[^A-Za-z0-9_-]+", "-", name).strip("-") or "PATTERN"
        lines.append(f"*{safe}, {label.replace(',', ' ')}")
        for fam in families:
            nums = [num(v) for v in fam[:5]] + [num(d) for d in fam[5]]
            lines.append(", ".join(nums))
    return "\n".join(lines) + "\n"


def clean_families(raw) -> list:
    """Families as a document stores them, read back defensively (the file
    may come from anywhere): ``[[angle, x0, y0, along, across, [dashes]]]``
    with every number finite and every spacing wide enough."""
    out = []
    for fam in raw if isinstance(raw, list) else ():
        try:
            angle, x0, y0, along, across = (float(v) for v in fam[:5])
            dashes = [float(d) for d in (fam[5] if len(fam) > 5 else [])
                      ][:_MAX_DASHES]
        except (TypeError, ValueError, IndexError):
            continue
        nums = [angle, x0, y0, along, across] + dashes
        if not all(math.isfinite(v) for v in nums) \
                or abs(across) < _MIN_SPACING:
            continue
        out.append([angle, x0, y0, along, across, dashes])
        if len(out) >= _MAX_FAMILIES:
            break
    return out


def hatch_marks(key: str, box, scale: float = 1.0, angle: float = 0.0,
                families=None, offset=None) -> dict:
    """The marks of pattern ``key`` over ``box`` = ``(x0, y0, x1, y1)`` in
    paper mm, ``scale`` times its size and turned ``angle`` degrees about
    the paper's origin: ``{"lines": [(x0, y0, x1, y1)], "dots": [(x, y)],
    "circles": [(x, y, r)], "polys": [[(x, y), …]], "closed": [[(x, y),
    …]]}`` — the caller clips them to the cut. ``families`` draws an
    imported pattern (its families, as :func:`parse_pat` gives them). A
    solid pattern has no marks: the caller fills. ``offset`` — ``(dx,
    dy)`` paper mm — moves the whole pattern."""
    if families is not None:
        return generate_hatch(key, box, scale, float(angle or 0), clean_families(families),
                              offset, None, None, None)
    return generate_hatch(key, box, scale, float(angle or 0) + _TURN.get(key, 0.0),
                          _FAMILIES.get(key, ()), offset, _SCATTERED.get(key),
                          _WAVES.get(key), FITTED.get(key))


#: The wood patterns, which a layer may draw with a cross where a piece is
#: cut across its grain (``Layer.hatch_cross``).
WOOD = tuple(k for t, keys in HATCH_GROUPS if t == "Wood" for k in keys
             if k != "wood_cross")
#: A piece is taken as cut across its grain — a stud, a joist, a beam in
#: section — when it is no longer than this, in real mm, and no more than
#: :data:`_ACROSS_RATIO` times as long as it is thick; longer, it is a
#: board or a beam cut along.
_ACROSS_LONGEST = 600.0
_ACROSS_RATIO = 6.0


def cut_across(band, scale_n: float = 100.0, depth=None) -> bool:
    """Whether a cut piece (``fit_band``'s band, paper mm, a sheet at
    1:``scale_n``) is a piece of timber cut across its grain. ``depth`` —
    how far the piece runs through the plane, square to it, paper mm (see
    core.hlr.solid_depth) — settles it from the 3D piece: timber runs the
    way it is longest, so a piece that goes on further through the plane
    than within it is cut across (a stud in a plan, a joist in a section),
    one longer within it is cut along. Without it, the cut's own shape
    guesses: short and stocky is across."""
    if band is None:
        return False
    length, thick = band[3], band[4]
    if thick <= 1e-9:
        return False
    if depth is not None:
        return float(depth) >= length
    real = length * max(1e-6, float(scale_n or 100.0))
    return real <= _ACROSS_LONGEST and length <= _ACROSS_RATIO * thick


def cross_marks(origin, along, across, length: float,
                thick: float) -> list:
    """A piece of timber cut across its grain: its two diagonals, from
    corner to corner of the rectangle ``length`` by ``thick``."""
    return fitted_marks("wood_cross", origin, along, across, length, thick)


def zigzag_marks(origin, along, across, length, thick, density=1.0):
    return fitted_marks("insulation_zigzag", origin, along, across, length, thick, density)


def batt_marks(origin, along, across, length, thick, density=1.0):
    return fitted_marks("insulation", origin, along, across, length, thick, density)
