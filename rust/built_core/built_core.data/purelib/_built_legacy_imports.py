"""One-release compatibility for plugin imports and launch environments."""
import os
import sys
from importlib.machinery import ModuleSpec, PathFinder


def inherit_environment():
    for key, value in tuple(os.environ.items()):
        if key.startswith("INGETRAZO_"):
            os.environ.setdefault("BUILT_" + key.removeprefix("INGETRAZO_"), value)


class LegacyImports:
    packages = {"ingetrazo_core": "built_core", "ingetrazo_render": "built_render",
                "ingetrazo_rfa": "built_rfa"}

    def find_spec(self, name, path=None, target=None):
        root, dot, suffix = name.partition(".")
        if root in self.packages:
            dest = self.packages[root] + dot + suffix
        elif root == "built_core" and dot and "." not in suffix:
            # Native registrations take precedence. Mirror Python plugin modules
            # without executing their classes a second time.
            if PathFinder.find_spec(name, path) is not None:
                return None
            from importlib import import_module
            core = import_module("core")
            dest = "core." + suffix
            if dest not in sys.modules and not PathFinder.find_spec(dest, core.__path__):
                return None
        else:
            return None
        spec = ModuleSpec(name, self)
        spec.loader_state = dest
        return spec

    def create_module(self, spec):
        return None

    def exec_module(self, module):
        from importlib import import_module
        spec = module.__spec__
        canonical = import_module(spec.loader_state)
        sys.modules[spec.name] = canonical
        # Embedded PyO3 modules can publish children without a package path.
        for child, obj in tuple(sys.modules.items()):
            if child.startswith(spec.loader_state + "."):
                sys.modules.setdefault(spec.name + child[len(spec.loader_state):], obj)


inherit_environment()
sys.meta_path.insert(0, LegacyImports())
