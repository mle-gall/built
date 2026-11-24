"""Smoke-test imports, the IFC schema plugins and installed application data."""
import importlib
import sys
from pathlib import Path


def check():
    root = Path(sys._MEIPASS)
    for name in ('numpy', 'openskp',
                 'manifold3d', 'ingetrazo_core', 'ingetrazo_render', 'ingetrazo_rfa',
                 'ifcopenshell'):
        module = importlib.import_module(name)
        if name == 'ifcopenshell':
            module.file(schema='IFC4')
    for rel in ('resources/textures/library.json', 'resources/components/components.json',
                'i18n/es.json', 'resources/icons/ingetrazo_256.png'):
        if not (root / rel).is_file():
            raise FileNotFoundError(root / rel)
    print('[built] native modules, IFC4 schema and bundle data OK', flush=True)
