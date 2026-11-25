#!/usr/bin/env bash
# The check to run BEFORE every release (Marco, 25-09-2026: «recuerda esto
# siempre antes de cada release pasar por pruebas rigurosas, además deberías
# guardar en un archivo tus resultados, para más adelante compararlos y si
# hubiera alguna fuga de memoria o fallas en el rendimiento cazarlos»).
#
#   scripts/release_check.sh <previous tag> <real .igz>
#   e.g. scripts/release_check.sh v0.5.2 ~/Descargas/plaza.igz
#
# Measure the release binary and replay input against the same document twice.
set -euo pipefail
PREV="${1:?previous tag}"
DOC="${2:?document path}"
HERE="$(cd "$(dirname "$0")/.." && pwd)"
DOC="$(cd "$(dirname "$DOC")" && pwd)/$(basename "$DOC")"
test -f "$DOC"
PY="${PYO3_PYTHON:-$HERE/.venv/bin/python}"
[[ -x "$PY" ]] || PY="$HERE/venv/bin/python"
export PYO3_PYTHON="$PY" RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}" CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
cd "$HERE"
VER="$("$PY" -c 'from built_core.version import __version__; print(__version__)')"
WORK="$(mktemp -d)"
trap 'git -C "$HERE" worktree remove --force "$WORK/prev" 2>/dev/null || true; rm -rf "$WORK"' EXIT
git worktree add --detach -q "$WORK/prev" "$PREV"
export HOME="$WORK/home" CFFIXED_USER_HOME="$WORK/home" BUILT_BACKGROUND=1 BUILT_SETTINGS_DIR="$WORK/settings"
mkdir -p "$HOME" "$BUILT_SETTINGS_DIR"
cat > "$WORK/session" <<'SCRIPT'
wait-open
wait 120
move 400 300
drag 400 300 600 300 middle 20
wheel 500 300 -40
pinch 500 300 0.1
state
quit
SCRIPT
for revision in prev head; do
  root="$HERE"
  [[ "$revision" == head ]] || root="$WORK/prev"
  if [[ ! -f "$root/rust/built_app/Cargo.toml" && "$revision" == prev ]]; then
    echo "Previous release predates the native shell. Recording a baseline."
    continue
  fi
  target="$WORK/target-$revision"
  cargo build --locked --release --target-dir "$target" --manifest-path "$root/rust/built_app/Cargo.toml"
  # The embedded renderer comes from this binary. Use the same revision
  # of the separately loaded document engine when timing its open.
  wheels="$WORK/wheels-$revision"
  "$PY" -m maturin build --release --locked -m "$root/rust/built_core/Cargo.toml" -o "$wheels"
  "$PY" -m pip install --no-index --no-deps --target "$WORK/engine-$revision" "$wheels"/built_core-*.whl
  for run in 1 2; do
    for mode in measure script; do
      args=(--measure)
      [[ "$mode" == measure ]] || args=(--script "$WORK/session")
      PYTHONPATH="$WORK/engine-$revision${PYTHONPATH:+:$PYTHONPATH}" BUILT_PERF=1 "$PY" scripts/isolated_launch.py "$target/release/built" --root "$root" \
        --venv "$(dirname "$(dirname "$PY")")" "${args[@]}" "$DOC" > "$WORK/$revision-$mode-$run.log"
    done
  done
done
cd "$HERE"
# In batches, one process each: the fast suite no longer fits in memory as
# a single run (07-10, >15 GB) — see scripts/pytest_batches.py.
# A failing test is recorded in the report (and named below), not a reason
# to throw the measurements away: the run used to stop here with nothing.
tests_status=0
FAST="$(QT_QPA_PLATFORM=offscreen "$PY" scripts/pytest_batches.py -m 'not slow' tests 2>"$WORK/fast.err" | tail -1)" || tests_status=1
SLOW="$(QT_QPA_PLATFORM=offscreen "$PY" scripts/pytest_batches.py -m slow tests 2>"$WORK/slow.err" | tail -1)" || tests_status=1
grep -hE "^(FAILED|ERROR)|CRASHED" "$WORK/fast.err" "$WORK/slow.err" || true

"$PY" - "$WORK" "$VER" "$PREV" "$DOC" "$FAST" "$SLOW" "$tests_status" <<'PYEOF'
import json, sys, datetime, os
work, ver, prev, doc, fast, slow, tests_status = sys.argv[1:]
from pathlib import Path

def records(revision, mode, label):
    values = []
    for log in sorted(Path(work).glob(f"{revision}-{mode}-*.log")):
        lines = [line for line in log.read_text().splitlines() if line.startswith(label)]
        if not lines:
            raise RuntimeError(f"{log} missing {label}")
        values.append(json.loads(lines[-1][len(label):]))
    return values

out = {"version": ver, "compared_with": prev, "date": datetime.date.today().isoformat(),
       "document": os.path.basename(doc), "tests": {"fast": fast, "slow": slow}}
for revision in ("head", "prev"):
    out[revision] = {"startup": records(revision, "measure", "[built] measure "),
                     "session": records(revision, "script", "[built] perf ")}
path = Path("benchmarks/results") / f"{ver}.json"
path.parent.mkdir(parents=True, exist_ok=True)
path.write_text(json.dumps(out, indent=2) + "\n")
print(path)
failed = bool(int(tests_status))
for revision in ("head", "prev"):
    for sample in out[revision]["startup"]:
        if (sample["first_frame_ms"] < 0 or sample["state"].get("errors")
                or Path(sample["state"].get("path") or "") != Path(doc)):
            failed = True
for key in ("frame_ms", "move_ms"):
    def best(revision):
        return min(sample[key]["median"] for sample in out[revision]["session"])
    head = best("head")
    if out["prev"]["session"]:
        previous = best("prev")
        print(f"{key}: {previous:.3f} -> {head:.3f} ms")
        failed |= head > previous * 1.25
sys.exit(int(failed))
PYEOF
