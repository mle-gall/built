# -*- mode: python ; coding: utf-8 -*-
"""Collect Built's embedded Python runtime, data files and MCP console helper.

Windows, Linux and macOS install the Rust launcher into the collected bundle.
Resource paths under _internal mirror the source tree.
"""
import os
import site
import sys
from pathlib import Path

ROOT = Path(SPECPATH).resolve().parent
from built_core.formats.built import BUILT_SUFFIX
built_extension = BUILT_SUFFIX[1:]

# ── Bundled assets — destinations MIRROR the repo layout ─────────────────────
datas = [
    # Render with Blender (#181): the script Blender runs on the job.
    ('resources/blender/*.py',     'resources/blender'),
    # The Built Link add-on: Open in Blender runs it, Install writes its zip.
    ('integrations/blender/built_link/*.py', 'integrations/blender/built_link'),
    ('integrations/blender/built_link/*.md', 'integrations/blender/built_link'),
    ('resources/icons/*.png',      'resources/icons'),
    ('resources/icons/*.ico',      'resources/icons'),
    ('resources/icons/mimetypes/*.ico', 'resources/icons/mimetypes'),
    ('resources/mime/*.xml',       'resources/mime'),
    ('resources/colors/*.json',    'resources/colors'),
    ('resources/textures/*.png',   'resources/textures'),
    ('resources/textures/library.json', 'resources/textures'),
    # Factory window layout (Marco's toolbar arrangement, harvested
    # 2026-08-31): fresh profiles restore it; user changes override it.
    ('resources/ui/*.state',       'resources/ui'),
    # The library itself — a whole tree, and it was MISSING: the Materials
    # tray read library.json and then found no images, so every category
    # came up empty in the packaged build (the Flatpak copies all of
    # resources/ and was fine, which is why it went unnoticed).
    ('resources/textures/library', 'resources/textures/library'),
    # Starter components are .igz since 0.3.7: one group per file with its
    # images packed inside, so the tray works with no network. (A stale glob
    # here is a HARD PyInstaller error, not a warning — the .glb set is gone.)
    ('resources/components/*.igz', 'resources/components'),
    ('resources/components/components.json', 'resources/components'),
    ('resources/components/people.json', 'resources/components'),
    ('resources/components/SOURCES.md', 'resources/components'),
    ('resources/components/*.png', 'resources/components'),
    ('resources/components/thumbs/*.png', 'resources/components/thumbs'),
    ('i18n/*.json',                'i18n'),
    # Bundled plugins (Extensions menu) — discovered by file path at runtime.
    ('plugins/*.py',               'plugins'),
    # Example extensions: shipped but NOT loaded — Extensions ▸ Example
    # extensions copies one into the user's plugins folder on request.
    # Whole folder: an example extension can be a package (CAM, PR #132).
    ('examples/extensions',        'examples/extensions'),
    # The MCP server (stdlib-only): `built --mcp` runs it by path, and
    # the console build below makes it a program of its own on Windows.
    ('scripts/built_mcp.py',   'scripts'),
]

# Optional trees (present today, tolerated if pruned later).
for opt_src, opt_dst in [
    ('resources/styles', 'resources/styles'),
    ('resources/fonts', 'resources/fonts'),
]:
    if (ROOT / opt_src).is_dir():
        datas.append((opt_src, opt_dst))

# The DWG satellite (LibreDWG dwg2dxf, see vendor/libredwg/SOURCES.md),
# found by formats/dwg_bridge.py at vendor/libredwg/bin inside the bundle
# (#101). Linux: the ELF in git. Windows: dwg2dxf.exe and its two DLLs,
# which build-windows.yml fetches from LibreDWG's own win64 release.
# macOS: built from the release tarball by release-macos.yml. A platform
# whose files are not there builds without DWG, and --check says so.
import sys as _sys


def _is_macho(path):
    """True when *path* begins with a Mach-O / universal binary magic.

    macOS builds its dwg2dxf from the LibreDWG tarball in release-macos.yml;
    the file committed to git is the LINUX ELF, so a plain macOS checkout
    must not bundle it (a dead 15 MB binary that also fooled --check).
    """
    try:
        head = path.read_bytes()[:4]
    except OSError:
        return False
    return head in (b"\xcf\xfa\xed\xfe", b"\xce\xfa\xed\xfe",
                    b"\xfe\xed\xfa\xcf", b"\xfe\xed\xfa\xce",
                    b"\xca\xfe\xba\xbe", b"\xbe\xba\xfe\xca")


_DWG_FILES = {
    "linux": ["dwg2dxf"],
    "win32": ["dwg2dxf.exe", "libredwg-0.dll", "libiconv-2.dll"],
    "darwin": ["dwg2dxf"],
}
for _plat, _names in _DWG_FILES.items():
    if _sys.platform.startswith(_plat):
        for _n in _names:
            _src = ROOT / "vendor/libredwg/bin" / _n
            if not _src.is_file():
                continue
            if _plat == "darwin" and not _is_macho(_src):
                print('spec: dwg2dxf is not a Mach-O — not bundled (macOS '
                      'DWG needs the LibreDWG build from release-macos.yml)')
                continue
            datas.append((f"vendor/libredwg/bin/{_n}", "vendor/libredwg/bin"))
        datas.append(("vendor/libredwg/SOURCES.md", "vendor/libredwg"))

# ── Hidden imports ───────────────────────────────────────────────────────────
from PyInstaller.utils.hooks import collect_submodules

# The solid tools in the Rust extension load this Python extension at runtime.
hiddenimports = ['manifold3d']
# truststore (#783): Python's HTTPS on Windows verifies through the system
# verifier; imported inside built_core.python_tls.https_context, and without it the
# extension manager silently falls back to Python's own store.
hiddenimports += ['truststore']
# Binary64 model values and the acceleration kernels share one extension.
import importlib.util as _ilu
if _ilu.find_spec('built_core') is None:
    raise SystemExit("Build built_core before packaging the application")
hiddenimports += ['core._native', 'built_core', '_built_file_packages']
# Collect the optional native .rfa family reader when installed.
omit_rfa = sys.platform == 'darwin' and os.environ.get('BUILT_PACKAGE_RFA') == '0'
if not omit_rfa and _ilu.find_spec('built_rfa') is not None:
    hiddenimports += collect_submodules('built_rfa')
if _ilu.find_spec('built_render') is None:
    raise RuntimeError('Build built_render with scripts/build_native.sh first')
hiddenimports += collect_submodules('built_render')

hiddenimports += [
    # The bundled plugins import these at RUN time, so static analysis never
    # sees them and they were left out: the AI assistant died on load with
    # "cannot import name 'ai' from 'core'" in every packaged build.
    'core.ai',
    # The recipe book both AI doors read. It reaches the bundle only
    # through a plugin (the assistant) and a by-path script (the MCP
    # server), so analysis never sees it — and its absence is SILENT: the
    # server falls back to a short string and the model goes back to
    # probing the API, which is the very waste it exists to stop.
    'core.ai_recipes',
    'core.bim',
    # The native model reader imports this Python package lazily.
    'openskp',
    'openskp.model',
    'openskp._core',
    'openskp.legacy',
    'openskp.vff',
    'openskp.parser',
    'openskp.transforms',
    'openskp.materials',
    'openskp.metadata',
    'openskp.triangulator',
    'openskp.create',
    'openskp.edit',
    'openskp.errors',
    'openskp.scene',
]

# ...and every other app module a by-path file imports, found by reading
# them rather than listed by hand. 0.5.6 shipped views.fold_section (new,
# imported only by the AI and Render plugins) outside every PyInstaller
# build, and those three extensions failed to load on Windows (#208).
def _app_imports(paths):
    import ast
    found = set()
    for path in paths:
        try:
            tree = ast.parse(Path(path).read_text(encoding='utf-8'))
        except (OSError, SyntaxError):
            continue
        for node in ast.walk(tree):
            if isinstance(node, ast.Import):
                names = [a.name for a in node.names]
            elif isinstance(node, ast.ImportFrom) and node.module and not node.level:
                names = [node.module] + [f"{node.module}.{a.name}" for a in node.names]
            else:
                continue
            for name in names:
                parts = name.split('.')
                if (ROOT.joinpath(*parts).with_suffix('.py').is_file()
                        or (ROOT.joinpath(*parts) / '__init__.py').is_file()):
                    found.add(name)
    return sorted(found)


hiddenimports += _app_imports(
    [*(ROOT / 'plugins').glob('*.py'),
     *(ROOT / 'examples/extensions').rglob('*.py'),
     ROOT / 'scripts/built_mcp.py'])
# The rest of openskp (export/*, _face_groups, instanced_scene, codegen…)
# plus its PACKAGE DATA: create.py loads ``_scaffold/blank_v17.skp`` via
# importlib.resources and PyInstaller never bundles non-Python files on its
# own. Without this every PyInstaller build (Windows exe, AppImage, tar)
# died on the .skp export with "[Errno 2] No such file or directory:
# …\\_internal\\openskp\\_scaffold\\blank_v17.skp" (reported from Windows,
# 0.4.1). The Flatpak was fine because it ships the whole site-packages.
from PyInstaller.utils.hooks import collect_data_files
hiddenimports += collect_submodules('openskp')
# ...except _scaffold/blank_v17.skp, the blank document openskp's writer
# builds its files on top of. Built does not distribute it and has no
# .skp export; main.py --check fails a frozen bundle that still carries it.
datas += [(src, dst) for src, dst in collect_data_files('openskp')
          if '_scaffold' not in src.replace('\\', '/')]
# openskp 1.3.0 triangulates with mapbox_earcut instead of Shapely, so the
# reader now pulls a NATIVE extension (_core*.so) that did not exist in the
# dependency tree before. ``import openskp`` fails outright without it, so
# a bundle that misses it is dead on arrival — the same shape of failure as
# the missing scaffold in 0.4.1. PyInstaller would probably follow the
# import on its own; "probably" is what cost us that release, so it is
# named here and ``main.py --check`` verifies it in CI.
hiddenimports += collect_submodules('mapbox_earcut')
# …and naming it was not enough. The Windows wheel is repaired with
# delvewheel: ``_core.pyd`` is linked against DLLs that live in a SIBLING
# directory of the package, ``site-packages/mapbox_earcut.libs/``, exactly
# like numpy.libs and shapely.libs. Those two work because
# pyinstaller-hooks-contrib ships hooks for them; mapbox_earcut has none,
# so the .pyd travelled without its dependencies and the bundle died with
# «DLL load failed while importing _core: The specified module could not
# be found» — which, because openskp imports it at module level, means no
# .skp import or export at all. Caught by ``--check`` on the v0.4.3
# tag, which is what that check is for.
#
# The DLLs go to the bundle root: PyInstaller puts sys._MEIPASS on the DLL
# search path, and delvewheel's own ``os.add_dll_directory`` preamble
# cannot help here because it resolves a path relative to a __file__ that
# does not exist in a frozen app.
from PyInstaller.utils.hooks import collect_dynamic_libs
binaries = collect_dynamic_libs('mapbox_earcut')
for _sp in site.getsitepackages() + [site.getusersitepackages()]:
    _sib = Path(_sp) / 'mapbox_earcut.libs'
    if _sib.is_dir():
        binaries += [(str(f), '.') for f in _sib.iterdir() if f.is_file()]
        break
print('spec: mapbox_earcut native files: %d' % len(binaries))

excludes = [
    'tkinter',
    'matplotlib',
    'pandas',
    'IPython',
    'jupyter',
    'numpy.tests',
    # Dev-only IFC validator: heavy, never imported by the app itself.
    'ifcopenshell',
    'pytest',
]

for package in ('core', 'formats', 'views', 'tools'):
    hiddenimports += collect_submodules(package)

if omit_rfa:
    excludes.append('built_rfa')

app_excludes = excludes.copy()
if sys.platform == 'darwin':
    app_excludes.remove('ifcopenshell')
    hiddenimports += collect_submodules('ifcopenshell')
    datas += collect_data_files('ifcopenshell')
    binaries += collect_dynamic_libs('ifcopenshell')
    datas += [('tests/fixtures/shell/slab-and-cube.igz', 'examples'),
              ('packaging/native_check.py', '.')]

a = Analysis(
    [str(ROOT / 'packaging/native_check.py')],
    pathex=[str(ROOT)],
    binaries=[(str(ROOT / src), dest) for src, dest in binaries],
    datas=[(str(ROOT / src), dest) for src, dest in datas],
    hiddenimports=hiddenimports,
    hookspath=[],
    hooksconfig={},
    runtime_hooks=[],
    excludes=app_excludes,
    noarchive=True,
)
# Keep the host GPU driver's X11, C++ runtime and GLib together (#6).
# Bundling the build runner's copies can break NVIDIA on newer systems.
_HOST_ONLY = {'libX11.so.6', 'libX11-xcb.so.1', 'libxcb-glx.so.0',
              'libstdc++.so.6', 'libgcc_s.so.1', 'libglib-2.0.so.0'}
if sys.platform.startswith('linux'):
    _before = len(a.binaries)
    a.binaries = [b for b in a.binaries
                  if Path(b[0]).name not in _HOST_ONLY]
    print('spec: dropped %d bundled X libraries (issue #6)'
          % (_before - len(a.binaries)))

# The DWG converter's DLLs live beside dwg2dxf.exe in vendor/libredwg/bin,
# where Windows looks for them when the .exe starts. PyInstaller also
# collects them into the bundle's root as dependencies of that .exe: a
# second copy of 31 MB nobody loads (#101).
if sys.platform == 'win32':
    _DWG_DLLS = {'libredwg-0.dll', 'libiconv-2.dll'}
    a.binaries = [b for b in a.binaries
                  if not (Path(b[0]).name.lower() in _DWG_DLLS
                          and Path(b[0]).parent == Path('.'))]

pyz =PYZ(a.pure, a.zipped_data)

icon = None
if sys.platform == 'win32':
    win_ico = ROOT / 'resources' / 'icons' / 'built.ico'
    if win_ico.exists():
        icon = str(win_ico)

exe = EXE(
    pyz,
    a.scripts,
    [],
    exclude_binaries=True,
    name='built',
    debug=False,
    bootloader_ignore_signals=False,
    strip=False,
    upx=False,
    console=False,              # GUI app, no terminal window
    disable_windowed_traceback=False,
    target_arch=None,
    codesign_identity=None,
    entitlements_file=None,
    icon=icon,
)

# ── built-mcp: a CONSOLE program for Claude Desktop / Claude Code ───────
# MCP clients talk over stdin/stdout; a windowed exe may have no std handles
# to give them, so Windows gets a console build of the stdlib-only server
# sharing this bundle's _internal/. Spawned with pipes it opens no window.
mcp_a = Analysis(
    [str(ROOT / 'scripts/built_mcp.py')],
    pathex=[str(ROOT)],
    binaries=[],
    datas=[],
    hiddenimports=['core.ai_recipes', 'core.agent_tools'],  # dependency-free catalog
    hookspath=[],
    runtime_hooks=[],
    excludes=excludes,
    noarchive=False,
)
mcp_pyz = PYZ(mcp_a.pure)
mcp_exe = EXE(
    mcp_pyz,
    mcp_a.scripts,
    [],
    exclude_binaries=True,
    name='built-mcp',
    debug=False,
    strip=False,
    upx=False,
    console=True,
    icon=icon,
)

# Refuse an add-on even if a data hook tries to package it under another name.
for destination, source, _ in [*a.datas, *mcp_a.datas]:
    if Path(source).resolve().is_relative_to((ROOT / 'addons').resolve()) or destination.endswith('.builtpack'):
        raise ValueError(f'Trade add-ons must be distributed separately: {destination}')

coll = COLLECT(
    exe,
    mcp_exe,
    a.binaries,
    a.zipfiles,
    a.datas,
    mcp_a.binaries,
    mcp_a.zipfiles,
    mcp_a.datas,
    strip=False,
    upx=False,
    upx_exclude=[],
    name='built',
)

# ── macOS: wrap the onedir COLLECT as a double-clickable .app ────────────────
# built-mcp rides along inside Contents/MacOS/ as a sibling binary (same
# onedir the other platforms ship); nothing launches it but a stdio-speaking
# MCP client that already knows its path, so it does not need its own bundle.
if sys.platform == 'darwin':
    _version = (ROOT / 'rust/built_core/VERSION').read_text().strip()
    mac_icon = ROOT / 'resources' / 'icons' / 'built.icns'
    app = BUNDLE(
        coll,
        name='Built.app',
        icon=str(mac_icon) if mac_icon.exists() else None,
        bundle_identifier='com.built.Built',
        info_plist={
            'CFBundleName': 'Built',
            'CFBundleDisplayName': 'Built',
            'CFBundleShortVersionString': _version,
            'CFBundleVersion': _version,
            'NSHighResolutionCapable': True,
            # PyInstaller's own Info.plist template defaults this to True,
            # which makes macOS treat the bundle as agent-only: no Dock icon,
            # no window, no menu bar — a GUI app must say so explicitly.
            'LSBackgroundOnly': False,
            'NSHumanReadableCopyright': 'GPL-3.0-or-later — Built contributors; based on IngeTrazo by Marco Sumari Tellez and IngeTrazo contributors',
            # .igz / .skp double-click-to-open, same association the other
            # platforms register (packaging/built.desktop, the .iss).
            'UTExportedTypeDeclarations': [{
                'UTTypeIdentifier': 'com.built.built-document',
                'UTTypeDescription': 'Built document',
                'UTTypeConformsTo': ['public.zip-archive'],
                'UTTypeTagSpecification': {'public.filename-extension': [built_extension]},
            }],
            'CFBundleDocumentTypes': [
                {
                    'CFBundleTypeName': 'Built document',
                    'CFBundleTypeExtensions': [built_extension],
                    'LSItemContentTypes': ['com.built.built-document'],
                    'CFBundleTypeRole': 'Editor',
                    'LSHandlerRank': 'Owner',
                },
                {
                    'CFBundleTypeName': 'Built document',
                    'CFBundleTypeExtensions': ['igz'],
                    'CFBundleTypeRole': 'Editor',
                    'LSHandlerRank': 'Owner',
                },
                {
                    'CFBundleTypeName': 'SKP document',
                    'CFBundleTypeExtensions': ['skp'],
                    'CFBundleTypeRole': 'Editor',
                    'LSHandlerRank': 'Alternate',
                },
            ],
        },
    )
