"""Bindings for the Python elevation sampler and export dialogs."""

from pathlib import Path

from georef.photomesh import PhotoMeshSampler
from georef.profile import profile_to_csv, sample_profile, selected_geopath
from views.inspector_bridge import _send


class ProfileBinding:
    def __init__(self, path):
        self.path = path
        self.surface = self.sampler = self.profile = self.signature = None
        self.dirty = True

    def changed(self):
        self.dirty = True

    def sync(self, view, vp):
        scene = vp.scene
        if self.path not in scene.geo_paths or scene.georef is None:
            action(view, vp, "close")
            return
        survey = getattr(scene, "photo_mesh", None)
        survey = survey if getattr(survey, "visible", False) else None
        if self.surface != (survey, scene.georef):
            self.surface = (survey, scene.georef)
            if survey:
                self.sampler = PhotoMeshSampler(survey, scene.georef)
            else:
                from georef.dem import DEMSampler

                self.sampler = DEMSampler(scene.georef)
                self.sampler.changed.connect(self.changed)
                self.sampler.changed.connect(view.update)
            self.dirty = True
        if self.signature != scene.version or self.dirty:
            self.dirty = False
            self.signature = scene.version
            self.profile = sample_profile(self.path.profile_points(), self.sampler)
            self.vertical = "odm" if survey else "dem"
            self.alt = scene.georef.alt
            _send(
                view,
                "terrain",
                dict(
                    name=self.path.name or "Path",
                    source="Field survey" if survey else "Global DEM",
                    samples=[[s.station, s.elevation] for s in self.profile.samples],
                ),
            )
            self.projected_sig = None
        matrix = vp.camera.projection_matrix() * vp.camera.view_matrix()
        sig = tuple(matrix.data()), view.width(), view.height()
        if sig != self.projected_sig:
            from ingetrazo_core import Vec3 as QVector3D

            _send(
                view,
                "projected",
                [
                    vp._world_to_pixel(vp.drape(QVector3D(s.x, s.y, 0)), matrix)
                    for s in self.profile.samples
                ],
            )
            self.projected_sig = sig


def action(view, vp, verb):
    binding = getattr(view, "_profile_binding", None)
    if verb == "close":
        view._profile_binding = None
        _send(view, "terrain", None)
    elif verb == "open":
        path = binding.path if binding else selected_geopath(vp.scene)
        if path is None or len(path.points) < 2 or vp.scene.georef is None:
            raise ValueError("Select a path and set the base map location first")
        view._profile_binding = ProfileBinding(path)
        view._profile_binding.sync(view, vp)
    elif binding and binding.profile:
        from views.filedialogs import file_dialogs

        ext = "csv" if verb == "csv" else "png"
        if ext == "png":
            data = bytes(view.renderer.ui_profile_png())
        else:
            data = profile_to_csv(binding.profile, binding.vertical, binding.alt).encode("utf-8")
        path, _ = file_dialogs.getSaveFileName(
            None, "Export terrain profile", "profile." + ext, "*." + ext
        )
        if path:
            Path(path).write_bytes(data)
