"""Turn the provisional steel profile sources into Library profile rows.

Each `addons/steel-profiles-<region>/source/` folder holds one CSV per
family (only the defining dimensions, as published) and `sources.json`
(where every value came from). This script draws every size with Built's
own parametric families, computes its area, mass (7850 kg/m³), second
moments and elastic moduli, checks them against the listed area and mass,
and writes the pack's `rows/<uid>.builtpreset` files and manifest:

    .venv/bin/python scripts/build_steel_profiles.py            # every steel pack
    .venv/bin/python scripts/build_steel_profiles.py steel-profiles-en
    .venv/bin/python scripts/build_steel_profiles.py --check    # compare, write nothing

A size whose computed area or mass is off by more than 1 % (2 % for sloped
flanges) stays out of the pack and is listed in `source/report.md` with the
reason. Every row is `status: provisional` until it is checked against the
purchased standard; see addons/steel-profiles-PROVENANCE.md.
"""
import argparse
import csv
import hashlib
import json
import math
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[1]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))
from scripts.build_addons import ADDONS, rehash  # noqa: E402

DENSITY = 7850.0  # kg/m³
TOLERANCE = 0.01
TOLERANCE_SLOPED = 0.02
SEGMENTS = 512
NUMBERS = ["h", "b", "tw", "tf", "r1", "r2", "slope", "mass", "area"]


def _sizes(path):
    with open(path, newline="", encoding="utf-8") as f:
        for row in csv.DictReader(f):
            size = {"designation": row["designation"].strip(), "status": row["status"], "source": row["source"]}
            for k in NUMBERS:
                size[k] = float(row[k]) if row[k] not in ("", None) else None
            yield size


def params(shape, s, family):
    """A size's dimensions as the parametric family's params (mm)."""
    p = {}
    if shape in ("i", "channel", "tee"):
        p = {"height": s["h"], "width": s["b"], "web": s["tw"], "flange": s["tf"], "radius": s["r1"] or 0}
        if s["r2"]:
            p["toe"] = s["r2"]
        if s["slope"]:
            p["slope"] = s["slope"]
            # Where the tabulated flange thickness is measured, from the tip.
            at = family.get("flange_at")
            if at == "b/4":
                p["flange_at"] = s["b"] / 4
            elif at == "b/2":
                p["flange_at"] = s["b"] / 2
            elif at == "(b-tw)/2":
                p["flange_at"] = (s["b"] - s["tw"]) / 2
    elif shape == "angle":
        p = {"height": s["h"], "width": s["b"], "web": s["tw"], "radius": s["r1"] or 0}
        if s["r2"]:
            p["toe"] = s["r2"]
    elif shape == "rhs":
        p = {"height": s["h"], "width": s["b"], "web": s["tw"], "radius": s["r1"] or 0}
        if s["r2"]:
            p["inner"] = s["r2"]
    elif shape == "chs":
        p = {"height": s["h"], "width": s["h"], "web": s["tw"]}
    else:
        raise ValueError(f"unknown shape {shape}")
    return {k: round(v, 4) for k, v in p.items()}


def slug(designation):
    """`IPE 200` → `ipe200`, `HSS3-1/2X3-1/2X3/16` → `hss3-1_2x3-1_2x3_16`."""
    text = designation.lower().replace("×", "x").replace("/", "_")
    return "".join(c for c in text if c.isalnum() or c in ".x-_")


def source_ids(cited):
    """`ob-ub;r1=bb-ub`: the row's source, then sources of single columns."""
    first, *rest = cited.split(";")
    return [first] + [part.split("=", 1)[1] for part in rest]


def design_params(shape, p, s):
    """The wall a tube's listed area was computed with (AISC: the design
    thickness, 0.93 × nominal for ERW), radii scaled with it."""
    if shape not in ("rhs", "chs") or not s["tf"] or s["tf"] == s["tw"]:
        return p
    k = s["tf"] / s["tw"]
    q = dict(p, web=s["tf"])
    for r in ("radius", "inner"):
        if r in q:
            q[r] = round(q[r] * k, 4)
    return q


def label(designation, family):
    text = designation.replace(" x ", " × ")
    if family.get("label_x"):
        text = text.replace("X", " × ")
    return f"{text} ({family['label_suffix']})" if family.get("label_suffix") else text


def section(props, listed_area, listed_mass):
    area = props["area"]
    return {
        "area_cm2": round(area / 100, 3),
        "mass_kg_m": round(area * 1e-6 * DENSITY, 3),
        "iy_cm4": round(props["i_xx"] / 1e4, 2),
        "iz_cm4": round(props["i_yy"] / 1e4, 2),
        "iu_cm4": round(props["i_u"] / 1e4, 2),
        "iv_cm4": round(props["i_v"] / 1e4, 2),
        "wy_cm3": round(props["w_xx"] / 1e3, 2),
        "wz_cm3": round(props["w_yy"] / 1e3, 2),
        "centroid_mm": [round(props["cx"], 2), round(props["cy"], 2)],
        "density": DENSITY,
        "listed": {"area_cm2": listed_area, "mass_kg_m": listed_mass},
    }


def evaluate(folder):
    """Every size of a pack folder: its row content, or why it stays out."""
    from built_core import command_protocol

    src = json.loads((folder / "source/sources.json").read_text(encoding="utf-8"))
    sources = src["sources"]
    kept, refused, stats, nominal_off = [], [], {}, []
    for family in src["families"]:
        shape = family["shape"]
        tol = TOLERANCE_SLOPED if family.get("sloped") else TOLERANCE
        worst = [0.0, 0.0]
        sizes = list(_sizes(folder / "source" / family["file"]))
        for s in sizes:
            for sid in source_ids(s["source"]):
                if sid not in sources:
                    raise ValueError(f"{family['file']}: {s['designation']} cites unknown source {sid}")
            key = src.get("keys", {}).get(f"{family['file']}:{s['designation']}") or \
                f"{src['prefix']}/{slug(s['designation'])}{family.get('key_suffix', '')}"
            p = params(shape, s, family)
            why = None
            try:
                props = command_protocol.profile_properties({"family": shape, "params": p}, SEGMENTS)
            except ValueError as e:
                props, why = None, f"the {shape} family cannot draw it ({e})"
            if props:
                design = design_params(shape, p, s)
                checked = props if design == p else command_protocol.profile_properties({"family": shape, "params": design}, SEGMENTS)
                area_cm2 = checked["area"] / 100
                mass = props["area"] * 1e-6 * DENSITY
                da = area_cm2 / s["area"] - 1 if s["area"] else None
                dm = mass / s["mass"] - 1 if s["mass"] else None
                if da is None and dm is None:
                    why = "no listed area or mass to check against"
                nominal = family.get("mass") == "nominal"
                for name, d, listed, ours in (("area", da, s["area"], area_cm2), ("mass", dm, s["mass"], mass)):
                    if d is not None:
                        worst[0 if name == "area" else 1] = max(worst[0 if name == "area" else 1], abs(d))
                        if abs(d) > tol and name == "mass" and nominal and da is not None:
                            # A designation's nominal mass (W 150 x 18) is not a computed value:
                            # the area decides, the deviation is reported.
                            nominal_off.append((s["designation"], family["file"], ours, listed, d))
                        elif abs(d) > tol and not why:
                            why = f"computed {name} {ours:.4g} vs listed {listed:g} ({d * 100:+.2f} %, limit {tol * 100:g} %)"
            entry = {"family": family, "size": s, "key": key, "params": p, "props": props,
                     "label": label(s["designation"], family)}
            note = src.get("notes", {}).get(f"{family['file']}:{s['designation']}") or family.get("left_out_note")
            if why and note:
                why = f"{why}. {note}"
            (refused if why else kept).append({**entry, "why": why})
        stats[family["file"]] = {"sizes": len(sizes), "worst_area": worst[0], "worst_mass": worst[1]}
    stats["_nominal"] = nominal_off
    return src, kept, refused, stats


def _canonical(row):
    return {k: v for k, v in row.items() if k not in ("rev", "hash")}


def rows_for(folder, src, kept):
    """The row files' content, reusing each key's uid and raising rev on change."""
    from built_core import command_protocol

    existing = {}
    for path in (folder / "rows").glob("*.builtpreset"):
        row = json.loads(path.read_text(encoding="utf-8"))
        existing[row["values"].get("profile", {}).get("library") if row.get("profile_size") else path.name] = row
    pack_name = json.loads((folder / "pack.json").read_text(encoding="utf-8"))["pack"]["name"]
    def series_of(key):
        # The same stem and suffix around the size number (`en/ipe` 240 ``,
        # `en/he` 200 `a`): what a junction rule's "± 1 catalogue step" counts,
        # as profile::series does for built-in sizes.
        at = next((i for i, c in enumerate(key) if c.isdigit()), len(key))
        rest = key[at:]
        end = next((i for i, c in enumerate(rest) if not (c.isdigit() or c == ".")), len(rest))
        return key[:at], rest[end:]
    by_family = {}
    for e in kept:
        by_family.setdefault((e["family"]["file"], series_of(e["key"])), []).append(e)
    out = []
    for fam_file, entries in by_family.items():
        order = sorted(entries, key=lambda e: (e["size"]["h"], e["size"]["b"] or 0, e["size"]["mass"] or 0))
        index = {e["key"]: i for i, e in enumerate(order)}
        for e in entries:
            i = index[e["key"]]
            series = [[n["key"], n["size"]["h"]] for n in order[max(0, i - 2):i + 3]]
            fam, s = e["family"], e["size"]
            ids = source_ids(s["source"])
            source = src["sources"][ids[0]]
            old = existing.get(e["key"])
            uid = old["uid"] if old else hashlib.sha256(f"built/steel-profiles/{e['key']}".encode()).hexdigest()[:16]
            row = {
                "uid": uid,
                "name": e["label"],
                "rev": 1,
                "pack": pack_name,
                "primitive": "sweep",
                "record": {},
                "locks": {},
                "values": {"profile": {"library": e["key"], "family": fam["shape"], "params": e["params"],
                                       "catalogue": {"name": e["label"], "series": series}}},
                "provenance": {"created_with": "Built", "status": s["status"], "source_id": s["source"],
                               "source": source["url"], "standard": fam["standard"],
                               "series": fam["series"],
                               **({"column_sources": {part.split("=")[0]: src["sources"][part.split("=")[1]]["url"]
                                                      for part in s["source"].split(";")[1:]}} if len(ids) > 1 else {})},
                "section": section(e["props"], s["area"], s["mass"]),
                "profile_size": True,
                "depends": [{"kind": "profile", "key": e["key"], "bundled": None}],
            }
            if old:
                row["rev"] = old.get("rev", 1) + (0 if _canonical(old) == _canonical(row) else 1)
            row["hash"] = command_protocol.pack_row_hash(row)
            out.append(row)
    keep = [r for k, r in existing.items() if not r.get("profile_size")]
    return keep, out


def report(folder, src, kept, refused, stats):
    lines = [f"# {src['title']}: generator report", "",
             "Written by scripts/build_steel_profiles.py. Every size is drawn with Built's parametric",
             "families; area and mass (7850 kg/m³) are computed from the drawing and compared with the",
             f"listed values (limit {TOLERANCE * 100:g} %, {TOLERANCE_SLOPED * 100:g} % for sloped flanges).", "",
             f"{len(kept)} sizes in the pack, {len(refused)} left out.", "",
             "| Family file | Sizes | Largest area deviation | Largest mass deviation |", "|---|---|---|---|"]
    for name, st in stats.items():
        if name == "_nominal":
            continue
        lines.append(f"| {name} | {st['sizes']} | {st['worst_area'] * 100:.2f} % | {st['worst_mass'] * 100:.2f} % |")
    if stats.get("_nominal"):
        lines += ["", "## Kept with a nominal mass more than the limit away", "",
                  "These families list the designation's nominal mass, not a computed one; their area matches.", "",
                  "| Size | Family file | Computed mass | Nominal mass | Deviation |", "|---|---|---|---|---|"]
        lines += [f"| {d} | {f} | {o:.4g} | {m:g} | {x * 100:+.2f} % |" for d, f, o, m, x in stats["_nominal"]]
    lines += ["", "## Left out", ""]
    if refused:
        lines += ["| Size | Family file | Why |", "|---|---|---|"]
        lines += [f"| {e['size']['designation']} | {e['family']['file']} | {e['why']} |" for e in refused]
    else:
        lines.append("None.")
    return "\n".join(lines) + "\n"


def generate(folder, check=False):
    src, kept, refused, stats = evaluate(folder)
    keep, rows = rows_for(folder, src, kept)
    text = report(folder, src, kept, refused, stats)
    wanted = {f"rows/{r['uid']}.builtpreset": json.dumps(r, indent=2, ensure_ascii=False) + "\n" for r in rows}
    stale = [p for p in (folder / "rows").glob("*.builtpreset")
             if json.loads(p.read_text(encoding="utf-8")).get("profile_size") and f"rows/{p.name}" not in wanted]
    differs = [n for n, t in wanted.items() if not (folder / n).is_file() or (folder / n).read_text(encoding="utf-8") != t]
    if (folder / "source/report.md").read_text(encoding="utf-8") != text if (folder / "source/report.md").is_file() else True:
        differs.append("source/report.md")
    if check:
        return differs + [str(p.relative_to(folder)) for p in stale], kept, refused
    for p in stale:
        p.unlink()
    for name, t in wanted.items():
        if (folder / name).is_file() and (folder / name).read_text(encoding="utf-8") == t:
            continue
        (folder / name).write_text(t, encoding="utf-8")
    (folder / "source/report.md").write_text(text, encoding="utf-8")
    manifest_path = folder / "pack.json"
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    kept_names = [f"rows/{r['uid']}.builtpreset" for r in keep]
    manifest["rows"] = [n for n in manifest["rows"] if n in kept_names] + \
        [n for n in kept_names if n not in manifest["rows"]] + list(wanted)
    manifest["pack"]["files"] = {n: h for n, h in manifest["pack"].get("files", {}).items() if n in manifest["rows"]}
    manifest_path.write_text(json.dumps(manifest, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    rehash(folder)
    return differs, kept, refused


PROVENANCE = ADDONS / "steel-profiles-PROVENANCE.md"


def provenance(summaries):
    """addons/steel-profiles-PROVENANCE.md: per pack, families, counts, sources, what replaces them."""
    out = ["# Steel profile packs: provenance", "",
           "Generated by scripts/build_steel_profiles.py from each pack's `source/sources.json` and",
           "`source/report.md`. **Every size in these packs is provisional**: its dimensions were",
           "transcribed from the public sources below (free previews of the standards, standards or",
           "industry bodies, mill catalogues), cross-checked where a second source exists",
           "(`source/crosscheck.md`), and drawn with Built's own parametric families, which compute",
           "every property. They stay provisional until each size is checked against the purchased",
           "standard named in the last column; replace a size by editing its family CSV and running the",
           "generator. No proprietary software catalogue was used. Pack content is CC0-1.0.", ""]
    for folder, src, kept, refused in summaries:
        counts = {}
        for e in kept:
            counts[e["family"]["file"]] = counts.get(e["family"]["file"], 0) + 1
        out += [f"## {src['title']} (`addons/{folder.name}`)", "", src.get("note", ""), "",
                f"{len(kept)} sizes; {len(refused)} left out (listed with the reason in `source/report.md`).", "",
                "| Family | Shape | Standard | Sizes | Sources | Provisional values | Replaced by |", "|---|---|---|---|---|---|---|"]
        for fam in src["families"]:
            ids = set()
            for e in kept:
                if e["family"]["file"] == fam["file"]:
                    ids.update(source_ids(e["size"]["source"]))
            flags = []
            if fam.get("sloped"):
                flags.append(f"flange slope and where tf is measured ({fam.get('flange_at', 'outstand middle')})")
            if fam["shape"] in ("i", "channel", "tee", "angle"):
                flags.append("root and toe radii")
            if fam["shape"] == "rhs":
                flags.append("corner radii")
            flags.append("all dimensions until checked")
            out.append(f"| {fam['series']} (`{fam['file']}`) | {fam['shape']} | {fam['standard']} | {counts.get(fam['file'], 0)} | "
                       f"{', '.join(sorted(ids)) or '-'} | {'; '.join(flags)} | {fam.get('replaced_by', '')} |")
        out += ["", "| Source | Title | Edition | Accessed | Columns used | Notes |", "|---|---|---|---|---|---|"]
        for sid, so in src["sources"].items():
            out.append(f"| `{sid}` | [{so['title']}]({so['url']}) | {so.get('edition', '')} | {so['accessed']} | {so.get('columns', '')} | {so.get('terms', '')} {so.get('kind', '')} |")
        if src.get("checks"):
            out += ["", "Cross-check sources: " + "; ".join(f"`{k}` {v.get('title', '')}" for k, v in src["checks"].items()) + "."]
        if src.get("missing"):
            out += ["", "Not covered yet: " + "; ".join(src["missing"]) + "."]
        out.append("")
    return "\n".join(out)


def folders(names=()):
    found = sorted(p.parent.parent for p in ADDONS.glob("steel-profiles-*/source/sources.json"))
    return [f for f in found if not names or f.name in names]


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("packs", nargs="*")
    parser.add_argument("--check", action="store_true", help="report what would change and exit non-zero")
    args = parser.parse_args()
    stale = False
    summaries = []
    for folder in folders(args.packs):
        differs, kept, refused = generate(folder, check=args.check)
        print(f"{folder.name}: {len(kept)} sizes, {len(refused)} left out" + (f", {len(differs)} files differ" if args.check else ""))
        stale |= bool(differs)
        summaries.append((folder, json.loads((folder / "source/sources.json").read_text(encoding="utf-8")), kept, refused))
    if not args.packs:
        text = provenance(summaries)
        if not PROVENANCE.is_file() or PROVENANCE.read_text(encoding="utf-8") != text:
            stale = True
            if not args.check:
                PROVENANCE.write_text(text, encoding="utf-8")
    sys.exit(1 if args.check and stale else 0)
