"""Smoke-test imports, the IFC schema plugins and installed application data."""
import importlib
import os
import sys
from pathlib import Path


def check():
    root = Path(sys._MEIPASS)
    for name in ('numpy', 'openskp',
                 'manifold3d', 'built_core', 'built_render',
                 'ifcopenshell'):
        module = importlib.import_module(name)
        if name == 'ifcopenshell':
            module.file(schema='IFC4')
    if os.environ.get("BUILT_PACKAGE_RFA") == "1":
        importlib.import_module("built_rfa")
    for rel in ('resources/textures/library.json', 'resources/components/components.json',
                'i18n/es.json', 'resources/icons/built_256.png'):
        if not (root / rel).is_file():
            raise FileNotFoundError(root / rel)
    print('[built] native modules, IFC4 schema and bundle data OK', flush=True)
