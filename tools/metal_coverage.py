"""Generate the Metal audit from the built-in window registry and transient tools."""
import ast
from pathlib import Path
import re


def registry(root):
    tree = ast.parse((root / 'views/main_window.py').read_text())
    from ingetrazo_render import window_tool_types
    result = dict(window_tool_types())
    # Tools handed straight to set_active_tool (component placement after
    # Insert / Link / the tray) have no toolbar key: they are listed under
    # their class name in snake case.
    for node in ast.walk(tree):
        if (isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute)
                and node.func.attr == 'set_active_tool' and node.args
                and isinstance(node.args[0], ast.Call)
                and isinstance(node.args[0].func, ast.Name)
                and node.args[0].func.id.endswith('Tool')):
            cls = node.args[0].func.id
            key = re.sub(r'(?<!^)(?=[A-Z])', '_', cls[:-4]).lower()
            result.setdefault(key, cls)
    solids = ast.parse((root / 'tools/solid_tools.py').read_text())
    for node in solids.body:
        if isinstance(node, ast.Assign) and any(isinstance(t, ast.Name) and t.id == 'SOLID_TOOLS' for t in node.targets):
            result.update((pair.elts[0].value, pair.elts[1].id) for pair in node.value.elts)
    return result


def table(root=None):
    from views.wgpu_tools import TOOL_DISPATCH
    root = Path(root) if root else Path(__file__).resolve().parents[1]
    sources = [p.read_text() for p in (root / 'tools').glob('*.py')]
    rows = ['| Registry key | Class | Metal status | GL hooks / preview dependencies |',
            '| --- | --- | --- | --- |']
    for key, cls in registry(root).items():
        status = 'native' if cls in TOOL_DISPATCH else 'inert with note'
        if key == 'section':
            status += ' — after the crate work'
        hooks = set()
        for source in sources:
            if 'class ' + cls + '(' not in source:
                continue
            bases = re.search(r'^class ' + cls + r'\(([^)]*)\)', source, re.M)
            mixins = bases.group(1).split(', ') if bases else []
            if 'RustTool' in mixins and len(mixins) > 1:
                # The viewport callbacks live on the tool's host mixin.
                source = '\n'.join(text for text in sources for base in mixins
                                   if base.endswith('Host') and 'class ' + base in text)
            if 'RustTool' in mixins:
                hooks.add('Rust state machine')
            hooks.update(re.findall(r'(?:viewport|vp)\.((?:pick_\w+|\w*preview\w*|_world_to_pixel|_pixel_to_ray))', source))
            if 'rubber_band_lines' in source:
                hooks.add('rubber_band_lines')
            if 'scale_box_state' in source:
                hooks.add('scale_box_state (screen handles)')
        rows.append(f"| {key} | {cls} | {status} | {', '.join(sorted(hooks)) or 'shared viewport / tool base'} |")
    return '\n'.join(rows)
