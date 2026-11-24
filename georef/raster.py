# SPDX-License-Identifier: GPL-3.0-or-later
"""Image exchange with extensions that still supply a Qt image."""
from ingetrazo_render import SurveyRaster


def as_raster(image):
    if image is None or isinstance(image, SurveyRaster):
        return image
    if image.isNull():
        return None
    rgba = image.convertToFormat(image.Format.Format_RGBA8888)
    return SurveyRaster(rgba.width(), rgba.height(), bytes(rgba.constBits()))
