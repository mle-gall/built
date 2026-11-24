#!/usr/bin/env bash
# scripts/linux_smoke.sh [both|x11|wayland] [output-directory]
set -euo pipefail
backend=${1:-both}
case "$backend" in x11|wayland|both) ;; *) echo "expected x11, wayland or both" >&2; exit 2;; esac
root=$(cd "$(dirname "$0")/.." && pwd)
out=${2:-$root/build/linux-smoke}
mkdir -p "$out"
out=$(cd "$out" && pwd)
cd "$root"
work=$(mktemp -d)
cleanup() {
    if [[ -s "$work/container" ]]; then docker rm -f "$(cat "$work/container")" >/dev/null 2>&1 || true; fi
    rm -rf "$work"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
# Include tracked working-tree edits. Keep local profiles and build products out.
mkdir "$work/context"
git ls-files -z | COPYFILE_DISABLE=1 tar -cf - --null -T - | tar -xf - -C "$work/context"
docker build -f "$work/context/packaging/linux-smoke/Dockerfile" --iidfile "$work/image" "$work/context"
docker run --rm --cidfile "$work/container" --user "$(id -u):$(id -g)" \
    -v "$out:/result" "$(cat "$work/image")" "$backend"
