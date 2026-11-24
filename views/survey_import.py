# SPDX-License-Identifier: GPL-3.0-or-later
"""Place a georeferenced raster through the window’s import command."""
from pathlib import Path

from ingetrazo_core import Vec3 as QVector3D
from core.i18n import tr
from views import chrome_dialogs as _dialogs
from views.filedialogs import file_dialogs


def import_orthophoto(window) -> None:
    """Import a GeoTIFF orthomosaic (a WebODM ``odm_orthophoto.tif``, a
    QGIS export) as a georeferenced reference image: the file's own
    pixel → UTM transform places it on the scene's datum at its true
    size and orientation, so the model is traced over the real ground.
    Display-only, like every reference image (invariant #4); locked on
    arrival so a stray drag never moves the survey."""
    from georef.datum import SceneDatum
    from georef.geotiff import (GeoTiffError, describe, read_info,
                                read_rgba, unsupported_reason)

    path_str, _ = file_dialogs.getOpenFileName(
        window, tr("Import orthomosaic"), "",
        tr("GeoTIFF (*.tif *.tiff);;All files (*)"))
    if not path_str:
        return
    path = Path(path_str)
    try:
        info = read_info(path)
    except (GeoTiffError, OSError) as exc:
        _dialogs.critical(window, tr("Import orthomosaic"), str(exc))
        return
    if not info.georeferenced:
        _dialogs.warning(
            window, tr("Import orthomosaic"),
            tr("{name} carries no georeference (no map transform or an "
               "unknown coordinate system). Import it with "
               "File ▸ Import ▸ Image and size it by hand.",
               name=path.name))
        return
    reason = unsupported_reason(info)
    if reason:
        _dialogs.critical(
            window, tr("Import orthomosaic"),
            tr("{name}: {reason} is not supported by this reader.",
               name=path.name, reason=reason))
        return

    scene = window.viewport.scene
    corners = info.corners_geodetic()
    datum = getattr(scene, "georef", None)
    datum_existed = datum is not None
    if datum is None:
        # The picture knows where it is — anchor the scene on its centre
        # rather than asking for coordinates the user would look up.
        lat = sum(c[0] for c in corners) / 4.0
        lon = sum(c[1] for c in corners) / 4.0
        datum = SceneDatum(lat, lon)

    dlg, cb = window._import_progress(tr("Importing {name}…", name=path.name))
    max_px = 8192
    try:
        max_px = max(1024, min(8192, int(window.viewport.max_texture_size())))
    except Exception:  # noqa: BLE001 — no GL context yet: keep the default
        pass
    def cancelled():
        return dlg.wasCanceled() or window.viewport is None or window.viewport.scene is not scene

    def reading(fraction):
        cb(fraction * 0.9, "Decoding the orthomosaic…")
        return not cancelled()

    from core.texture import cache_image
    from ingetrazo_core import survey_webp
    try:
        rgba, factor = read_rgba(info, max_px=max_px, progress=reading)
        cb(0.92, "Storing the picture…")
        if cancelled():
            dlg.close()
            return
        cached = cache_image(survey_webp(rgba), f"{path.stem}.webp", "imported")
    except (GeoTiffError, ValueError, OSError, MemoryError) as exc:
        if cancelled():
            dlg.close()
            return
        _dialogs.close_failed(dlg, exc)
        _dialogs.critical(window, tr("Import orthomosaic"), str(exc))
        return
    dlg.close()

    # Place it: the plane's origin is the picture's bottom-left, u its
    # width, v its height — the corners come in that order, already
    # turned by the datum's north angle. On the datum plane (Z = 0),
    # like the base map.
    from core.image_plane import ImagePlane
    from core.history import AddImagePlaneCommand
    bl, br, _tr, tl = [datum.geodetic_to_local(lat, lon)
                       for lat, lon in corners]
    u, v = br - bl, tl - bl
    aspect = v.length() / u.length() if u.length() > 1e-9 else 1.0
    plane = ImagePlane(str(cached), bl, u, v, aspect=aspect,
                       name=path.stem, locked=True)
    if not datum_existed:
        scene.georef = datum
    window.viewport.history.execute(AddImagePlaneCommand(plane))
    if not datum_existed:
        lo, hi = QVector3D(bl), QVector3D(bl)
        for c in plane.corners():
            lo = QVector3D(min(lo.x(), c.x()), min(lo.y(), c.y()), 0.0)
            hi = QVector3D(max(hi.x(), c.x()), max(hi.y(), c.y()), 0.0)
        window._set_survey_capture(datum, lo, hi)
        window.viewport.camera.set_view("top")
        window.viewport.camera.fit_to(lo, hi)
    tray = getattr(window, "georef_tray", None)
    if tray is not None:
        tray.on_scene_changed()
    window.viewport.update()
    w_m, h_m = u.length(), v.length()
    window.statusBar().showMessage(
        tr("Imported {name} — {w:.0f} × {h:.0f} m, {info}{reduced}",
           name=path.name, w=w_m, h=h_m, info=describe(info),
           reduced=(tr(", reduced {k}×", k=factor) if factor > 1 else "")),
        8000)
