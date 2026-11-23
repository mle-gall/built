"""Upload the document's display-only imagery into the shared wgpu scene."""
import numpy as np
from PySide6.QtCore import Qt
from PySide6.QtGui import QImage


def _vertices(positions, uvs, origin, triangles=None):
    positions, uvs = np.asarray(positions), np.asarray(uvs)
    if triangles is not None:
        indices = np.asarray(triangles).ravel()
        positions, uvs = positions[indices], uvs[indices]
    return np.column_stack((positions - origin, uvs)).astype(np.float32).ravel()


def _texture(image, fallback, limit):
    if image is None or image.isNull():
        return 1, 1, np.array((*fallback, 255), np.uint8)
    if max(image.width(), image.height()) > limit:
        image = image.scaled(limit, limit, Qt.KeepAspectRatio, Qt.SmoothTransformation)
    image = image.convertToFormat(QImage.Format_RGBA8888)
    return image.width(), image.height(), np.frombuffer(bytes(image.constBits()), np.uint8)


def preview_texture(path, limit):
    return _texture(QImage(path).mirrored(), (245, 242, 236), limit)


def sync(view, style):
    """Keep per-layer GPU allocations across frames, including peer/capture cameras."""
    source = view.source_viewport
    scene = source.scene if source is not None else view.scene
    terrain = getattr(scene, 'terrain', None)
    tiles = getattr(scene, 'tile_layer', None)
    survey = getattr(scene, 'photo_mesh', None)
    images = getattr(scene, 'image_planes', ())
    shared = view._shared
    cached = getattr(shared, '_reference_cache', None)
    if not (terrain or tiles or survey or images or cached):
        return
    from views.wgpu_peer import raw_renderer
    renderer = raw_renderer(view.renderer)
    origin = tuple(view.packed.origin)
    scope = (renderer, scene, origin)
    old = cached[1] if cached and cached[0] == scope else {}
    layers, updates = {}, []
    revision = getattr(source, '_reference_revision', 0)

    def changed(key, signature, image=None):
        layers[key] = (signature, image.cacheKey() if image is not None else None)
        return layers[key] != old.get(key)

    def upload(key, geometry, image, fallback=(255, 255, 255), opacity=1.,
               fade=(0., 0., 0., 0.), depth=True):
        texture = _texture(image, fallback, renderer.max_texture_size())
        updates.append((key, geometry, *texture, opacity, fade, depth))

    datum = getattr(scene, 'georef', None)
    if terrain is not None and terrain.visible:
        if len(terrain.triangles) and changed('terrain', (id(terrain), revision), terrain.texture_image):
            geometry = _vertices([p.toTuple() for p in terrain.vertices], terrain.uvs,
                                 origin, terrain.triangles)
            upload('terrain', geometry, terrain.texture_image, (140, 153, 133))
    elif tiles is not None and tiles.visible and datum is not None:
        coverage = (id(tiles), tiles.zoom, tuple(map(tuple, tiles.patches)),
                    tuple(datum.to_dict().items()))
        if getattr(shared, '_tile_coverage', None) != coverage:
            shared._reference_tiles = tiles.flat_tiles(datum)
            shared._tile_coverage = coverage
        for x, y in shared._reference_tiles:
            addr = (x, y, tiles.zoom)
            image = tiles.images.get(addr)
            if image is None:
                image = source._disk_cached_tile(tiles.source, *addr)
                if image is None:
                    source._schedule_tile_requests()
                    continue
                tiles.images[addr] = image
            key = f'tile/{tiles.source.id}/{tiles.zoom}/{x}/{y}'
            if changed(key, (coverage, tiles.opacity), image):
                positions, uvs = zip(*tiles.quad_local(datum, x, y))
                geometry = _vertices([p.toTuple() for p in positions], uvs, origin)
                upload(key, geometry, image, opacity=tiles.opacity, depth=False)
    if survey is not None and survey.visible and scene.entity_visible(survey):
        for index, material in enumerate(survey.materials):
            key = f'photo/{index}'
            image = survey.images.get(index)
            if material.count and changed(key, (id(survey), revision, material.start, material.count), image):
                triangles = survey.triangles[material.start:material.start + material.count]
                geometry = _vertices(survey.vertices, survey.uvs, origin, triangles)
                upload(key, geometry, image, (158, 153, 143))
    if images:
        amount = source._edit_rest_fade(None) if source is not None else 0.
        fade = (*style.get('fade_color', (1., 1., 1.)), amount)
        for i, plane in enumerate(images):
            if amount is None or not scene.entity_visible(plane):
                continue
            corners = tuple(p.toTuple() for p in plane.corners())
            key = f'image/{i}'
            if changed(key, (plane.path, revision, corners, plane.opacity, fade)):
                image = QImage(plane.path)
                if image.isNull():
                    del layers[key]
                    continue
                geometry = _vertices(corners, [(0, 1), (1, 1), (1, 0), (0, 0)],
                                     origin, [0, 1, 2, 0, 2, 3])
                upload(key, geometry, image, opacity=plane.opacity, fade=fade,
                       depth=plane.opacity >= .999)
    if updates or (cached and list(cached[1]) != list(layers)):
        renderer.set_reference_layers(list(layers), updates)
    shared._reference_cache = (scope, layers) if layers else None
