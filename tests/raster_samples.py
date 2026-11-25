"""Generate and inspect RGBA test images with the production Rust codec."""
import numpy as np
from built_render import SurveyRaster


def solid_raster(width, height, color):
    rgba = tuple(color)
    if len(rgba) == 3:
        rgba += (255,)
    return SurveyRaster(width, height, bytes(rgba) * (width * height))


def pixels(image):
    return np.frombuffer(image.rgba(), np.uint8).reshape(image.height, image.width, 4)
