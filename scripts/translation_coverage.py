#!/usr/bin/env python3
"""Refresh the shipped source inventory with --write. Otherwise check and report.

Run the report from the checkout with the native modules installed:
    .venv/bin/python -m scripts.translation_coverage

A nonblank entry counts even when its spelling matches English (e.g. "OK").
Retired catalogue entries remain available for contributors.
"""
import argparse
import ast
import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
INVENTORY = ROOT / "rust/built_core/src/core_bulk/translation_sources.json"
# Literal arguments to the translator, PyO3 translator calls and native controls.
CALL = re.compile(
    r'(?:\b(?:trf?|t|translate|translated)\s*\(\s*(?:py\s*,\s*)?|'
    r'\b(?:tr|tr_fn|t|translate)\s*\.\s*call1?\s*\(\s*\(\s*|'
    r'(?:call_method1?\s*\(\s*"tr"\s*,|getattr\("tr"\)\?\.call1?\()\s*\(\s*|'
    r'\b(?:label|button|hint_text|on_hover_text|RichText::new)\(\s*|'
    r'kit::(?:caption|quiet|primary|secondary|empty_state)\(\s*ui,\s*)$'
)
RUST_TOKEN = re.compile(
    r"""//[^\n]*|/\*.*?\*/|'(?:\\.|[^'\\\n])'|r(?P<hash>\#*)"(?P<raw>.*?)"(?P=hash)|"(?:\\.|[^"\\])*" """.strip(),
    re.S,
)


def message_literals(node):
    if isinstance(node, ast.Constant) and isinstance(node.value, str):
        yield node.value
    elif isinstance(node, ast.IfExp):
        yield from message_literals(node.body)
        yield from message_literals(node.orelse)


def inventory():
    keys = set().union(*(json.loads(p.read_text("utf8")) for p in (ROOT / "i18n").glob("*.json")))
    keys.discard("_comment")
    used = set()
    for folder in ("plugins", "views", "core", "formats"):
        for path in (ROOT / folder).rglob("*.py"):
            tree = ast.parse(path.read_text("utf8"))
            for node in ast.walk(tree):
                if isinstance(node, ast.Constant) and isinstance(node.value, str) and node.value in keys:
                    used.add(node.value)
                if isinstance(node, ast.Call) and node.args:
                    name = getattr(node.func, "id", getattr(node.func, "attr", ""))
                    if name == "tr":
                        used.update(message_literals(node.args[0]))

    for src in (ROOT / "rust").glob("*/src"):
        for path in src.rglob("*.rs"):
            if path.name == "gallery.rs":
                continue
            code = path.read_text("utf8").split("#[cfg(test)]\nmod tests")[0]
            for m in RUST_TOKEN.finditer(code):
                if m.group("raw") is not None:
                    value = m.group("raw")
                elif not m[0].startswith('"'):
                    continue
                else:
                    try:
                        value = json.loads(m[0])
                    except ValueError:
                        continue
                if value in keys or CALL.search(code[max(0, m.start()-180):m.start()]):
                    used.add(value)
    for directory in [ROOT / "resources", *(ROOT / "rust").glob("*/src")]:
        for path in directory.rglob("*.json"):
            if path == INVENTORY:
                continue
            for literal in re.findall(r'"(?:\\.|[^"\\])*"', path.read_text("utf8")):
                value = json.loads(literal)
                if value in keys:
                    used.add(value)
    rows = json.loads((ROOT / "rust/built_render/src/preference_rows.json").read_text("utf8"))
    for groups in rows.values():
        for _, title, fields in groups:
            used.add(title)
            for field in fields:
                used.update(field[k] for k in ("label", "tip", "suffix", "restart") if k in field)
                used.update(label for _, label in field.get("options", []))
    return sorted(s for s in used if s.strip())


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--write", action="store_true")
    parser.add_argument("--inventory-only", action="store_true",
                        help="compare the inventory with the sources and stop (no native modules needed)")
    args = parser.parse_args()
    sources = inventory()
    if args.write:
        INVENTORY.write_text(json.dumps(sources, indent=1, ensure_ascii=False) + "\n", encoding="utf8")
    else:
        recorded = json.loads(INVENTORY.read_text("utf8"))
        if sources != recorded:
            added, gone = sorted(set(sources) - set(recorded)), sorted(set(recorded) - set(sources))
            for sign, strings in (("+", added), ("-", gone)):
                for text in strings[:20]:
                    print(f"  {sign} {text}")
            raise SystemExit(f"UI source inventory changed ({len(added)} new, {len(gone)} gone). "
                             f"Run: python scripts/translation_coverage.py --write")
        if args.inventory_only:
            print(f"UI source inventory up to date ({len(sources)} strings).")
            raise SystemExit(0)
        from built_core.i18n import language_coverage
        print("Language  Translated / Sources  Coverage")
        for code, (count, total) in language_coverage().items():
            print(f"{code:8}  {count:10} / {total:<7}  {count / total:.1%}")
