# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Seekable scratch storage for one large IGZ save, never a document cache."""
import json
import tempfile

from functools import partialmethod
from ingetrazo_core import Fragment, igz_json_chunks as json_chunks, igz_stream_store, igz_stream_encode

_CHUNK = 1024 * 1024


class SaveStream(dict):
    """Keep geometry blocks and completed mesh JSON on disk, one mesh at a time."""

    def __init__(self, *, built=False, compatibility=False, destination=None):
        self.built = built
        self.compatibility = compatibility
        self.destination = destination
        self.displacement_mm = 0.0
        super().__init__()  # ZIP geometry member names -> scratch ranges
        self.file = tempfile.TemporaryFile()
        self.blobs = {}
        self.resolved = {}
        self.missing = 0

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.file.close()

    store = partialmethod(igz_stream_store)
    encode = partialmethod(igz_stream_encode)
    json = partialmethod(igz_stream_encode, mesh=True)
    document = partialmethod(igz_stream_encode)


_encoder = json.JSONEncoder(separators=(",", ":"), allow_nan=False)
