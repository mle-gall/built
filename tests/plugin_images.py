"""Generate a small native raster for plugin image controls."""
from built_render import SurveyRaster


def write_png(path, width, height):
    pixels = bytes((220, 40, 40, 255)) * (width * height)
    path.write_bytes(SurveyRaster(width, height, pixels).png())
