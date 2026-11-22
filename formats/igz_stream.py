# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Seekable scratch storage for one large IGZ save, never a document cache."""
import json
import tempfile

_CHUNK = 1024 * 1024


class Fragment:
    """An immutable byte range; the owning save closes the scratch file."""

    def __init__(self, file, offset, size):
        self.file, self.offset, self.size = file, offset, size

    def chunks(self):
        offset, remaining = self.offset, self.size
        while remaining:
            self.file.seek(offset)
            data = self.file.read(min(_CHUNK, remaining))
            if not data:
                raise OSError("Truncated IGZ scratch data")
            yield data
            offset += len(data)
            remaining -= len(data)


class SaveStream(dict):
    """Keep geometry blocks and completed mesh JSON on disk, one mesh at a time."""

    def __init__(self):
        super().__init__()  # ZIP geometry member names -> scratch ranges
        self.file = tempfile.TemporaryFile()
        self.blobs = {}
        self.resolved = {}
        self.missing = 0

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.file.close()

    def store(self, chunks):
        # A document can read existing ranges of this SAME file while being
        # appended. Seek explicitly on both paths, and batch small JSON tokens.
        start = self.file.seek(0, 2)
        size = 0
        pending = bytearray()
        for chunk in chunks:
            if len(chunk) >= _CHUNK:
                self.file.seek(0, 2)
                size += self.file.write(pending)
                pending.clear()
                size += self.file.write(chunk)
                continue
            pending.extend(chunk)
            if len(pending) >= _CHUNK:
                self.file.seek(0, 2)
                size += self.file.write(pending)
                pending.clear()
        if pending:
            self.file.seek(0, 2)
            size += self.file.write(pending)
        return Fragment(self.file, start, size)

    def json(self, value):
        return self.store(json_chunks(value, mesh=True))

    def document(self, value):
        return self.store(json_chunks(value))


_encoder = json.JSONEncoder(separators=(",", ":"), allow_nan=False)


def json_chunks(value, *, mesh=False):
    """Splice scratch ranges verbatim and let the standard encoder format values.

    Completed meshes contain plain records. Encode their arrays in batches of
    256 records. The final document can contain fragments anywhere in a group
    hierarchy, so traverse those records individually.
    """
    if isinstance(value, Fragment):
        yield from value.chunks()
    elif isinstance(value, dict):
        yield b"{"
        for i, (key, item) in enumerate(value.items()):
            if i:
                yield b","
            if isinstance(key, str):
                yield _encoder.encode(key).encode("utf-8") + b":"
            else:
                # Preserve the encoder's conversion of non-string object keys.
                header = _encoder.encode({key: None})
                yield header[1:-5].encode("utf-8")
            yield from json_chunks(item, mesh=mesh)
        yield b"}"
    elif isinstance(value, (list, tuple)):
        yield b"["
        if mesh or all(not isinstance(v, (dict, list, tuple, Fragment)) for v in value):
            for start in range(0, len(value), 256):
                if start:
                    yield b","
                batch = _encoder.encode(value[start:start + 256])
                yield batch[1:-1].encode("utf-8")
        else:
            for i, item in enumerate(value):
                if i:
                    yield b","
                yield from json_chunks(item)
        yield b"]"
    else:
        yield _encoder.encode(value).encode("utf-8")
