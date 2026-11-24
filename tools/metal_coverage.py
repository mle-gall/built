"""Generate the Metal audit from the built-in window registry and transient tools."""
import ast
from pathlib import Path
import re


def registry(root):
    from ingetrazo_render import window_tool_types
    result = dict(window_tool_types())
    solids = ast.parse((root / 'tools/solid_host.py').read_text())
    for node in solids.body:
        if isinstance(node, ast.Assign) and any(isinstance(t, ast.Name) and t.id == 'SOLID_TOOLS' for t in node.targets):
            result.update((pair.elts[0].value, pair.elts[1].id) for pair in node.value.elts)
    return result


def table(root=None):
    from ingetrazo_core import tool_preview_hooks
    from views.wgpu_tools import TOOL_DISPATCH
    root = Path(root) if root else Path(__file__).resolve().parents[1]
    classes = {}
    for file in (root / 'tools').glob('*.py'):
        source = file.read_text()
        lines = source.splitlines()
        for node in ast.parse(source).body:
            if isinstance(node, ast.ClassDef):
                classes[node.name] = (f"tools.{file.stem}", [ast.unparse(base) for base in node.bases],
                                      "\n".join(lines[node.lineno - 1:node.end_lineno]))
    rows = ['| Registry key | Class | Metal status | GL hooks / preview dependencies |',
            '| --- | --- | --- | --- |']
    for key, cls in registry(root).items():
        status = 'native' if cls in TOOL_DISPATCH else 'inert with note'
        if key == 'section':
            status += ' — after the crate work'
        hooks = set()
        seen = set()
        pending = [cls]
        while pending:
            name = pending.pop()
            if name in seen or name not in classes:
                continue
            seen.add(name)
            if name == "RustTool":
                hooks.add("Rust state machine")
                continue
            if name == "Tool":
                continue
            module, bases, source = classes[name]
            hooks.update(tool_preview_hooks(root, module, name))
            pending.extend(bases)
            hooks.update(re.findall(r'(?:viewport|vp)\.((?:pick_\w+|\w*preview\w*|_world_to_pixel|_pixel_to_ray))', source))
            if 'rubber_band_lines' in source:
                hooks.add('rubber_band_lines')
            if 'scale_box_state' in source:
                hooks.add('scale_box_state (screen handles)')
        rows.append(f"| {key} | {cls} | {status} | {', '.join(sorted(hooks)) or 'shared viewport / tool base'} |")
    return '\n'.join(rows)
