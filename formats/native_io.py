# SPDX-License-Identifier: GPL-3.0-or-later
"""Bindings shared by the IGZ/Built document reader and writer."""
import os
import zipfile


from functools import partialmethod
from ingetrazo_core import document_native_io as native_io, document_archive_read


class DocumentArchive(zipfile.ZipFile):
    """Use the same open descriptor for directory checks and native reads."""

    read = partialmethod(document_archive_read)
