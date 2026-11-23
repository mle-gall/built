# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Fuzzy search over short labels, after Blender's F3 menu search.

A port of Blender's ``source/blender/blenlib/intern/string_search.cc``
(GPL-2.0-or-later, © Blender Authors), the matcher behind its F3 box. Each
word of the query must find its own word in the item, trying in turn:

1. a word that STARTS with it  («ori» → Orient) — 10 points, 9 when the
   word is in the menu path rather than in the command's own name;
2. the INITIALS of consecutive words, several letters per word allowed
   («rf» → Reverse Faces, «seboulo» → Select Boundary Loop) — 4 / 3;
3. a word it matches with a few TYPING ERRORS («orinet» → Orient; one
   error per 8 letters, plus one) — 3 minus the errors.

A word of the item serves one word of the query only, and every query
word out of order costs a point. Among the best matches the one with the
shortest name comes first; with one letter typed or none, the ones used
last do. Unlike Blender, accents and case are ignored everywhere.
"""
from __future__ import annotations

import unicodedata
from functools import lru_cache

from ingetrazo_core import (SearchItem, command_score as score,
                            damerau_levenshtein, fuzzy_errors, rank_commands)

GROUP_SEP = "▸"


@lru_cache(maxsize=1 << 13)
def fold(text: str) -> str:
    """Lower case, no accents: «Rotación» and «rotacion» are one word.
    Remembered: the same labels are folded each time a box opens."""
    decomposed = unicodedata.normalize("NFKD", text or "")
    return "".join(c for c in decomposed
                   if not unicodedata.combining(c)).casefold()


class Item(SearchItem):
    """Index the translated path and name with the native command matcher."""

    def __new__(cls, groups):
        return super().__new__(cls, [(fold(g), len(g)) for g in groups if g and g.strip()])


def words_of(text):
    return list(Item([text]).words)


def rank(query: str, entries, recent=None) -> list:
    entries = list(entries)
    short = len(query.strip()) <= 1
    ranked = rank_commands(fold(query), short, [
        (aliases, recent(payload) if recent is not None and short else -1)
        for payload, aliases in entries])
    return [entries[i][0] for i in ranked]
