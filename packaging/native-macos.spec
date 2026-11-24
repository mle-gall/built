"""Share the classic bundle's library, icon, document types and data manifest."""
from pathlib import Path
from PyInstaller.utils.hooks import collect_data_files, collect_dynamic_libs, collect_submodules

SPECPATH = str(Path(SPECPATH).parent)
classic_analysis = Analysis

def Analysis(scripts, **options):
    optional = {"PySide6", "shiboken6"}
    options["excludes"] = [*options.get("excludes", ()), *sorted(optional)]
    options["hiddenimports"] = [name for name in options.get("hiddenimports", ())
                               if name.partition(".")[0] not in optional]
    if scripts == ['main.py']:
        # CPython's regular filesystem loader and the classic bootloader
        # can both import this tree. No unpack or subprocess at startup.
        options['noarchive'] = True
        options['excludes'].remove('ifcopenshell')
        options['datas'] += collect_data_files('ifcopenshell')
        options['binaries'] += collect_dynamic_libs('ifcopenshell')
        options['hiddenimports'] += collect_submodules('ifcopenshell')
        for package in ('core', 'tools', 'formats', 'views', 'app_shell'):
            options['hiddenimports'] += collect_submodules(package)
        options['datas'] += [
            ('tests/fixtures/shell/slab-and-cube.igz', 'examples'),
            ('packaging/native_check.py', '.'),
        ]
    root = Path(SPECPATH)
    for key in ('datas', 'binaries'):
        options[key] = [(str(root / src), dest) for src, dest in options[key]]
    return classic_analysis([str(root / script) for script in scripts], **options)

exec(compile((Path(SPECPATH) / 'ingetrazo.spec').read_bytes(), 'ingetrazo.spec', 'exec'))
