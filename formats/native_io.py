# SPDX-License-Identifier: GPL-3.0-or-later
"""Bindings shared by the IGZ/Built document reader and writer."""
import os
import zipfile


def native_io():
    from core import _native
    if os.environ.get("INGETRAZO_RUST_IO") == "0":
        return None
    module = _native.available()
    return module if hasattr(module, "FileGroups") else None


class DocumentArchive(zipfile.ZipFile):
    """Use the same open descriptor for directory checks and native reads."""

    def read(self, name, pwd=None):
        reader = getattr(native_io(), "igz_read_member", None)
        if reader is None:
            return super().read(name, pwd)
        info = name if isinstance(name, zipfile.ZipInfo) else self.getinfo(name)
        with self.open(info, pwd=pwd) as member:
            # open() checks the local header, flags and overlapping entries.
            offset = getattr(member, "_orig_compress_start", None)
            if offset is not None and not info.flag_bits & 1:
                blob = reader(self.fp.fileno(), offset, info.compress_size,
                              info.file_size, info.compress_type, info.CRC)
                if blob is not None:
                    return blob
            return member.read()
