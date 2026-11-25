"""Locate Built's native file packages, including first imports by plugins."""
import sys


class _FilePackages:
    names = {"formats", "georef", "ingetrazo_core.formats", "ingetrazo_core.georef"}

    def find_spec(self, fullname, path=None, target=None):
        if fullname not in self.names:
            return None
        from importlib.machinery import ModuleSpec, PathFinder
        spec = ModuleSpec(fullname, self, is_package=True)
        if fullname == "formats":
            # Keep the source or frozen package directories containing formats.ifc.
            disk = PathFinder.find_spec(fullname)
            if disk and disk.submodule_search_locations:
                spec.submodule_search_locations.extend(disk.submodule_search_locations)
        return spec

    def create_module(self, spec):
        return None

    def exec_module(self, module):
        from importlib import import_module
        name = module.__name__
        if name.startswith("ingetrazo_core."):
            sys.modules[name] = import_module(name.rsplit(".", 1)[1])
            return
        import ingetrazo_core
        if name == "formats":
            ingetrazo_core.install_codec_modules(module)
        else:
            ingetrazo_core.register_survey(module)
        sys.modules["ingetrazo_core." + name] = module
        setattr(ingetrazo_core, name, module)


sys.meta_path.insert(0, _FilePackages())
