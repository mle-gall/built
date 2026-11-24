"""Translate glTF data channels to the document's shared-UV map recipe."""
from pathlib import Path

import numpy as np
from ingetrazo_render import SurveyRaster

from core.texture import cache_image


def _png(pixels, name):
    h, w, _ = pixels.shape
    return str(cache_image(SurveyRaster(w, h, pixels.tobytes()).png(), name, "pbr"))


def _pixels(image):
    return np.frombuffer(image.rgba(), np.uint8).reshape(image.height, image.width, 4).copy()


def import_recipe(mat, textures, paths):
    mr = mat.get("pbrMetallicRoughness", {})
    recipe = {"metallic": mr.get("metallicFactor", 1.0),
              "roughness": mr.get("roughnessFactor", 1.0)}
    def path(info):
        if info:
            source = textures[info["index"]].get("source")
            if source is not None and 0 <= source < len(paths):
                return paths[source]
        return None

    packed = path(mr.get("metallicRoughnessTexture"))
    ao = path(mat.get("occlusionTexture"))
    if packed and packed == ao:
        recipe["orm_map"] = {"path": packed}
    elif packed:
        image = SurveyRaster.open(packed)
        if image is not None:
            pixels = _pixels(image)
            for channel, key in ((1, "roughness_map"), (2, "metallic_map")):
                grey = pixels.copy()
                grey[:, :, :3] = pixels[:, :, channel:channel+1]
                grey[:, :, 3] = 255
                recipe[key] = {"path": _png(grey, key + ".png")}
    if ao:
        recipe["ao_strength"] = mat["occlusionTexture"].get("strength", 1.0)
        if ao != packed:
            recipe["ao_map"] = {"path": ao}
    normal = path(mat.get("normalTexture"))
    if normal:
        recipe["normal_map"] = {"path": normal}
        recipe["normal_scale"] = mat["normalTexture"].get("scale", 1.0)
    return recipe


def export_maps(recipe):
    """Return normal, metallic/roughness and occlusion image paths."""
    def path(key):
        return (recipe.get(key) or {}).get("path")

    normal = path("normal_map")
    if normal and recipe["normal_map"].get("flip_y"):
        image = SurveyRaster.open(normal)
        if image is not None:
            pixels = _pixels(image)
            pixels[:, :, 1] = 255 - pixels[:, :, 1]
            normal = _png(pixels, Path(normal).stem + "-opengl.png")
    packed = path("orm_map")
    if not packed:
        layers = [(i, SurveyRaster.open(src)) for i, key in ((1, "roughness_map"), (2, "metallic_map"))
                  if (src := path(key))]
        layers = [(i, im) for i, im in layers if im is not None]
        if layers:
            w = max(im.width for _, im in layers)
            h = max(im.height for _, im in layers)
            pixels = np.full((h, w, 4), 255, np.uint8)
            for channel, im in layers:
                pixels[:, :, channel] = _pixels(im.resized(w, h))[:, :, 0]
            packed = _png(pixels, "metallic-roughness.png")
    return normal, packed, path("orm_map") or path("ao_map")
