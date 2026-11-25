#!/usr/bin/env bash
# Install the Built launcher + icons for the current user (Linux).
# On Wayland/GNOME the dock and app-grid icon come from the .desktop entry
# and the hicolor icon theme — setWindowIcon alone is not enough.
#
# It also registers branded *document* icons for the file types Built
# works with (.igz / .dae / .skp) via a freedesktop MIME package. This is
# purely cosmetic: it makes the file manager show the icons, it does NOT
# change which program opens those files.
#
#   scripts/install_desktop.sh              install
#   scripts/install_desktop.sh --uninstall  remove everything it installed
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

APPS="$HOME/.local/share/applications"
ICONS_HICOLOR="$HOME/.local/share/icons/hicolor"
MIME_DIR="$HOME/.local/share/mime"
MIME_PKG="$MIME_DIR/packages/built.xml"
APP_SIZES=(16 32 48 64 128 256 512)
# freedesktop MIME icon names for each document type (see resources/mime).
MIME_ICONS=(application-x-built model-vnd.collada+xml application-vnd.sketchup.skp)

refresh_caches() {
  command -v update-desktop-database >/dev/null && \
    update-desktop-database "$APPS" 2>/dev/null || true
  command -v update-mime-database >/dev/null && \
    update-mime-database "$MIME_DIR" 2>/dev/null || true
  command -v gtk-update-icon-cache >/dev/null && \
    gtk-update-icon-cache -f "$ICONS_HICOLOR" 2>/dev/null || true
}

if [[ "${1:-}" == "--uninstall" ]]; then
  rm -f "$APPS/built.desktop" "$MIME_PKG"
  for size in "${APP_SIZES[@]}"; do
    rm -f "$ICONS_HICOLOR/${size}x${size}/apps/built.png"
  done
  for name in "${MIME_ICONS[@]}"; do
    find "$ICONS_HICOLOR" -name "${name}.png" -path '*/mimetypes/*' -delete 2>/dev/null || true
  done
  refresh_caches
  echo "Built desinstalado del lanzador."
  exit 0
fi

# ── Launcher ────────────────────────────────────────────────────────────────
mkdir -p "$APPS"
binary="${BUILT_BINARY:-${CARGO_TARGET_DIR:-$ROOT/rust/built_app/target}/release/built}"
if [[ ! -x "$binary" ]]; then
  echo "Missing executable $binary. Build built_app in release mode or set BUILT_BINARY." >&2
  exit 1
fi
binary="$(cd "$(dirname "$binary")" && pwd)/$(basename "$binary")"
while IFS= read -r line; do
  if [[ "$line" == "Exec=built "* ]]; then
    printf 'Exec="%s" %s\n' "$binary" "${line#Exec=built }"
  else
    printf '%s\n' "$line"
  fi
done < "$ROOT/packaging/built.desktop" > "$APPS/built.desktop"

# ── Application icon (dock / app grid) ──────────────────────────────────────
for size in "${APP_SIZES[@]}"; do
  dir="$ICONS_HICOLOR/${size}x${size}/apps"
  mkdir -p "$dir"
  cp "$ROOT/resources/icons/built_${size}.png" "$dir/built.png"
done
# The scalable SVG too, since 2026-08-07 there is one: a theme that prefers
# scalable gets it crisp at any size the shell asks for, instead of upscaling
# the 512 PNG.
if [ -f "$ROOT/resources/icons/built.svg" ]; then
  mkdir -p "$ICONS_HICOLOR/scalable/apps"
  cp "$ROOT/resources/icons/built.svg" "$ICONS_HICOLOR/scalable/apps/built.svg"
fi

# ── Document icons for .igz / .dae / .skp ───────────────────────────────────
# Copy the hicolor mimetype PNGs and register the MIME package so the file
# manager paints the branded icon on those files.
HICOLOR_SRC="$ROOT/resources/icons/hicolor"
if [ -d "$HICOLOR_SRC" ] && [ -f "$ROOT/resources/mime/built.xml" ]; then
  cp -r "$HICOLOR_SRC/." "$ICONS_HICOLOR/"
  mkdir -p "$(dirname "$MIME_PKG")"
  cp "$ROOT/resources/mime/built.xml" "$MIME_PKG"
fi

refresh_caches

# ── Default opener: double-click opens Built ────────────────────────────
# .igz is our own format; .skp now opens natively (openskp backend), so both
# get Built as the default handler. .dae stays "Open with" only — we do
# not steal it from Blender & friends.
if command -v xdg-mime >/dev/null; then
  xdg-mime default built.desktop application/x-built 2>/dev/null || true
  xdg-mime default built.desktop application/vnd.sketchup.skp 2>/dev/null || true
fi

echo "Built instalado en el lanzador. Búscalo como 'Built'."
echo "Doble clic en .igz y .skp abre Built; .dae queda en 'Abrir con'."
