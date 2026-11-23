"""About credits. Scrolling and hover are exercised in the Rust kit."""
from pathlib import Path
from types import SimpleNamespace

from ingetrazo_render import AboutReview

from core.i18n import tr
from views.report_sheets import About


def test_everyone_is_in_the_credits(monkeypatch):
    sheet = About()
    ctx = SimpleNamespace(win=SimpleNamespace())
    spec = sheet.build(ctx).wire()
    rows = next(b["credits"] for b in spec["content"]["blocks"] if b["kind"] == "credits")
    opened = []
    monkeypatch.setattr("webbrowser.open", opened.append)
    for row, (name, role, url, starred) in zip(rows, AboutReview.contributors(), strict=True):
        assert (row["name"], row["role"], row["starred"]) == (name, tr(role), starred)
        if url:
            sheet.act(ctx, "verb", [row["verb"]])
            assert opened[-1] == url
    assert {"Pedro Caeiro", "Rafael García Rodríguez", "Ahsan Mehmood",
            "dafrobozao", "Félix Riestra", "Sherod Taylor"} <= {r["name"] for r in rows}


def test_every_contributor_in_authors_is_in_the_credits():
    import re
    authors = (Path(__file__).resolve().parents[1] / "AUTHORS").read_text()
    handles = set(re.findall(r"github\.com/([\w.-]+)\)", authors))
    links = {link.rsplit("/", 1)[-1] for _, _, link, _ in AboutReview.contributors() if link}
    assert not handles - links - {"tuxiasumari"}
    sheet = About()
    ctx = SimpleNamespace(win=SimpleNamespace())
    sheet.build(ctx)
    sheet.act(ctx, "verb", ["authors"])
    expanded = sheet.build(ctx).wire()["content"]["blocks"]
    assert any("Later contributors may add their names below." == b.get("text") for b in expanded)


def test_every_credit_line_is_translated():
    import json
    root = Path(__file__).resolve().parents[1] / "i18n"
    lines = [what for _, what, _, _ in AboutReview.contributors()]
    lines.append("Thanks as well to all the authors of the extensions catalogue")
    for path in sorted(root.glob("*.json")):
        if path.stem != "en":
            catalog = json.loads(path.read_text(encoding="utf-8"))
            assert not [line for line in lines if not catalog.get(line)], path.stem
