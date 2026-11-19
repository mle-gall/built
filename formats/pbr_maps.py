"""Translate glTF data channels to the document's shared-UV map recipe."""
from pathlib import Path

import numpy as np
from PySide6.QtCore import QBuffer, QIODevice, Qt
from PySide6.QtGui import QImage

from core.texture import cache_image


def _png(pixels, name):
    h, w, _ = pixels.shape
    image = QImage(pixels.data, w, h, 4*w, QImage.Format_RGBA8888)
    buf = QBuffer()
    buf.open(QIODevice.WriteOnly)
    if not image.save(buf, "PNG"):
        raise ValueError(f"Could not encode {name}")
    return str(cache_image(bytes(buf.data()), name, "pbr"))


def _pixels(image):
    rgba = image.convertToFormat(QImage.Format_RGBA8888)
    return np.frombuffer(rgba.constBits(), np.uint8).reshape(
        rgba.height(), rgba.width(), 4).copy()


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
        image = QImage(packed)
        if not image.isNull():
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
        image = QImage(normal)
        if not image.isNull():
            pixels = _pixels(image)
            pixels[:, :, 1] = 255 - pixels[:, :, 1]
            normal = _png(pixels, Path(normal).stem + "-opengl.png")
    packed = path("orm_map")
    if not packed:
        layers = [(i, QImage(src)) for i, key in ((1, "roughness_map"), (2, "metallic_map"))
                  if (src := path(key))]
        layers = [(i, im) for i, im in layers if not im.isNull()]
        if layers:
            w = max(im.width() for _, im in layers)
            h = max(im.height() for _, im in layers)
            pixels = np.full((h, w, 4), 255, np.uint8)
            for channel, im in layers:
                pixels[:, :, channel] = _pixels(im.scaled(
                    w, h, Qt.IgnoreAspectRatio, Qt.SmoothTransformation))[:, :, 0]
            packed = _png(pixels, "metallic-roughness.png")
    return normal, packed, path("orm_map") or path("ao_map")
