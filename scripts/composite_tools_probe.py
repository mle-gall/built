"""Bounded, synthetic linked-document tool probe; never opens user files.
Run with the project's Python: scripts/composite_tools_probe.py OUT.json.
PROBE_PANELS=1 exposes the docks and waits for debounced updates after edits.
PROBE_PROFILE=group/dimension/click writes a cProfile of the last matching
handler. PROBE_GROUPS and PROBE_LINES override the workload for smoke runs.
"""

import os
import sys
import tempfile
import shutil
import threading
import time
import json
from collections import Counter
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
os.environ["INGETRAZO_WGPU"] = "1"
os.environ["INGETRAZO_RUST"] = "1"
from scripts.open_path_probe import rss_now_mb, rss_peak_mb
from PySide6.QtCore import QSettings, Qt, QPointF, QEvent
from PySide6.QtGui import QMouseEvent
from ingetrazo_core import Vec3 as QVector3D, Matrix4 as QMatrix4x4
from PySide6.QtWidgets import QApplication

folder = Path(tempfile.mkdtemp(prefix="composite-tools-probe-"))
QSettings.setDefaultFormat(QSettings.IniFormat)
for scope in (QSettings.UserScope, QSettings.SystemScope):
    QSettings.setPath(QSettings.IniFormat, scope, str(folder))
from core import autosave

autosave.autosave_dir = lambda: folder
autosave.pending = lambda *a, **k: None
main_thread = threading.get_ident()
samples = Counter()
phase = ""


def watch():
    start = time.monotonic()
    next_rss = 0
    while True:
        now = time.monotonic()
        if now > next_rss:
            rss = rss_now_mb()
            if (rss and rss > 3000) or now - start > 900:
                print("ABORT: RSS/time budget", rss, flush=True)
                os._exit(2)
            next_rss = now + 0.5
        if phase:
            f = sys._current_frames().get(main_thread)
            stack = []
            while f:
                stack.append(
                    f"{Path(f.f_code.co_filename).name}:{f.f_code.co_name}:{f.f_lineno}"
                )
                f = f.f_back
            samples[(phase, " <- ".join(stack[:8]))] += 1
        time.sleep(0.002)


threading.Thread(target=watch, daemon=True).start()
app = QApplication([])
from core.scene import Scene
from core.mesh import Mesh
from core.group import Group
from core import links
from formats import igz
from core.dimension import Dimension
from core.guide import Guide
from core.layers import Layer, add_folder
from core.materials import Material

n = int(os.environ.get("PROBE_GROUPS", "12000"))
host = Scene()
for index in range(200):
    tag_folder = add_folder(host, f"Zone {index}")
    host.layers.append(Layer(f"Tag {index}", folder=tag_folder))
for i in range(20):
    p = QVector3D(-10, 10 + i, 0) if i else QVector3D(0, 0, 0)
    host.guides.append(Guide(p))
    host.dimensions.append(Dimension(p, p + QVector3D(1, 0, 0), QVector3D(0, 1, 0)))

for source_id in range(4):
    source = Scene()
    paint = Material(
        f"Finish {source_id}", (0.4, 0.6, 0.8), pbr={"metallic": 0.2, "roughness": 0.7}
    )
    source.materials[paint.name] = paint
    mesh = Mesh()
    for i in range(256):
        x, y = i % 16, i // 16
        face = mesh.add_face(
            [
                QVector3D(x, y, 0),
                QVector3D(x + 1, y, 0),
                QVector3D(x + 1, y + 1, 0),
                QVector3D(x, y + 1, 0),
            ]
        )
        face.attrs.update(paint.face_attrs())
    leaves = []
    for i in range(source_id, n, 4):
        g = Group(mesh, name=f"Part {i}")
        g.xform = QMatrix4x4()
        g.xform.translate(100 + (i % 128) * 18, 100 + (i // 128) * 18, 0)
        leaves.append(g)
    for depth in range(3 + source_id % 3):
        parents = []
        for i in range(0, len(leaves), 8):
            group = Group(name=f"Assembly {depth}-{i}")
            group.adopt(leaves[i : i + 8])
            parents.append(group)
        leaves = parents
    source.groups.extend(leaves)
    path = folder / f"source-{source_id}.igz"
    igz.save_scene(source, path)
    host.groups.append(links.make_link(host, path))

small = Group(name="editable")
small.component = False
for target, x in ((host.mesh, 0), (small.mesh, 6)):
    target.add_face(
        [
            QVector3D(x, 0, 0),
            QVector3D(x + 2, 0, 0),
            QVector3D(x + 2, 2, 0),
            QVector3D(x, 2, 0),
        ]
    )
host.groups.append(small)
igz.save_scene(host, folder / "host.igz")
del source, host
from views import prompts
from views.layers_panel import LayersPanel
from views.structure_panel import StructureDock
from views.tray import PartsPanel, EntityInfoPanel, ScenesPanel
from views.extension_api import ExtensionApp
from plugins import solid_inspector
from functools import wraps

# Timings include only refresh callbacks the active layout actually invokes.
panel_times = {}


def instrument(cls, method):
    original = getattr(cls, method)

    @wraps(original)
    def measured(self, *args, **kwargs):
        start = time.perf_counter()
        try:
            return original(self, *args, **kwargs)
        finally:
            key = cls.__name__ + "." + method
            panel_times[key] = (
                panel_times.get(key, 0) + (time.perf_counter() - start) * 1000
            )

    setattr(cls, method, measured)


for cls, method in (
    (LayersPanel, "on_scene_changed"),
    (LayersPanel, "_refresh_after_change"),
    (LayersPanel, "_count_slice"),
    (LayersPanel, "refresh"),
    (StructureDock, "on_scene_changed"),
    (StructureDock, "refresh"),
    (PartsPanel, "refresh"),
    (PartsPanel, "on_scene_changed"),
    (EntityInfoPanel, "refresh"),
    (ScenesPanel, "refresh"),
    (solid_inspector.SolidInspectorPanel, "refresh"),
):
    instrument(cls, method)
from views.main_window import MainWindow

win = MainWindow(defer_extensions=True)
win.resize(1100, 750)
win.setWindowFlag(Qt.WindowStaysOnTopHint, True)
win.setAttribute(Qt.WA_ShowWithoutActivating, True)
win.show()
win.open_path(folder / "host.igz")
vp = win.viewport
view = win._doc.stand_in
win._leave_empty_area()
win._doc_stack.setCurrentWidget(view)
view.show()


def settle(seconds):
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        app.processEvents()
        time.sleep(0.005)


settle(3)
small = next(g for g in vp.scene.groups if g.name == "editable")
solid_inspector.setup(ExtensionApp(win, "solid_inspector"))
panels_open = os.environ.get("PROBE_PANELS") == "1"
if panels_open:
    # Expose all requested docks. Collapse the middle sections in Properties
    # so that Entity Info and Parts receive real visible-panel refreshes.
    for panel in (win.tray.materials, win.tray.components):
        panel.parentWidget()._btn.setChecked(False)
    for dock in (
        win.layers_dock,
        win.structure_dock,
        win.tray,
        win.scenes_dock,
        win._solid_inspector_dock,
    ):
        dock.setFloating(True)
        dock.move(win.screen().availableGeometry().topLeft())
        dock.show()
    settle(1)
    win.tray.widget().ensureWidgetVisible(win.tray.parts)
    settle(0.5)
prompts.get_multiline_text = lambda *args: ("Construction", True)
results = {}
frames = []
resident_revision = vp.history.revision
set_scene = view.set_scene


def observe_scene(*args, **kwargs):
    global resident_revision
    result = set_scene(*args, **kwargs)
    resident_revision = vp.history.revision
    return result


view.set_scene = observe_scene


def presented(*args):
    if resident_revision == vp.history.revision:
        frames.append(time.perf_counter())


view.frameReady.connect(presented)
clock = time.perf_counter


def record(kind, action):
    global phase
    phase = scenario + "/" + kind
    revision = vp.history.revision
    frames.clear()
    panel_times.clear()
    t = clock()
    if os.environ.get("PROBE_PROFILE") == phase:
        import cProfile

        profiler = cProfile.Profile()
        profiler.runcall(action)
        profiler.dump_stats(str(Path(sys.argv[1]).with_suffix(".prof")))
    else:
        action()
    elapsed = (clock() - t) * 1000
    phase += "/frame"
    # A GPU pick may synchronously render the pre-edit scene inside the handler.
    frames.clear()
    view.update()
    deadline = clock() + 1
    while not frames and clock() < deadline:
        app.processEvents()
    frame_ms = (frames[0] - t) * 1000 if frames else None
    revisions = vp.history.revision - revision
    if revisions and panels_open:
        # Layers debounces counts for 300 ms. Structure builds in slices.
        settle(0.35)
        deadline = clock() + 5
        while (
            win.layers_dock.panel._count_timer.isActive()
            or win.structure_dock._pending_refresh is not None
        ) and clock() < deadline:
            settle(0.015)
    measurements.append(
        dict(
            event=kind,
            ms=elapsed,
            revisions=revisions,
            frame_ms=frame_ms,
            panels_ms=dict(panel_times),
        )
    )
    phase = ""


def click_point(x, y):
    pixel = view._world_to_pixel(QVector3D(origin + x, y, 0))
    point = QPointF(*pixel)
    for kind, typ, button, buttons, handler in (
        ("hover", QEvent.MouseMove, Qt.NoButton, Qt.NoButton, view.mouseMoveEvent),
        (
            "click",
            QEvent.MouseButtonPress,
            Qt.LeftButton,
            Qt.LeftButton,
            view.mousePressEvent,
        ),
        (
            "release",
            QEvent.MouseButtonRelease,
            Qt.LeftButton,
            Qt.NoButton,
            view.mouseReleaseEvent,
        ),
    ):
        event = QMouseEvent(typ, point, point, button, buttons, Qt.NoModifier)
        record(kind, lambda: handler(event))


count = int(os.environ.get("PROBE_LINES", "50"))
for context, origin in (("loose", 0), ("group", 6)):
    if context == "group":
        vp.begin_group_edit(small)
        view.set_scene(vp.scene)
    view.camera.set_view("top")
    view.camera.target = QVector3D(origin + 1, 1, 0)
    view.camera.distance = 12
    view.update()
    settle(1)
    for name in (
        "polyline",
        "separate",
        "rectangle",
        "circle",
        "arc",
        "guide",
        "point",
        "dimension",
        "text",
        "offset",
    ):
        scenario = context + "/" + name
        key = {
            "polyline": "line",
            "separate": "line",
            "guide": "tape",
            "point": "tape",
        }.get(name, name)
        win._activate_tool(key)
        tool = view.tools.tool
        tool.on_cancel(view.tools)
        if name == "point":
            tool._toggle_mode(view.tools)
        measurements = results[scenario] = []
        if name == "polyline":
            # Walk a zigzag in clear space, panning every four segments.
            # Separating successive runs prevents snapping to an earlier tool's work.
            y = 8 if context == "loose" else -8
            initial = vp.history.revision
            for i in range(count + 6):
                if vp.history.revision - initial >= count:
                    break
                if i % 4 == 0:
                    view.camera.target = QVector3D(origin + i + 1, y, 0)
                    view.update()
                    app.processEvents()
                click_point(i, y + (i % 2) * 0.8)
            view.camera.target = QVector3D(origin + 1, 1, 0)
            view.update()
            app.processEvents()
        elif name == "separate":
            for i in range(count):
                x, y = -2.5 + (i % 10) * 0.72, -2.2 + (i // 10) * 0.8
                click_point(x, y)
                click_point(x + 0.38, y + 0.28)
                tool.on_cancel(view.tools)
        else:
            paths = {
                "rectangle": [(-2, 1.5), (-1, 2.4)],
                "circle": [(3, 0), (3.6, 0)],
                "arc": [(2.5, 2.3), (3.8, 2.4), (3.2, 3.1)],
                "guide": [(1, 0), (0.7, -1.3)],
                "point": [(0, 0), (-1.3, -0.8)],
                "dimension": [(0, 0), (2, 0), (1, -0.8)],
                "text": [(2, 2), (3, 3)],
                "offset": [(1, 0.3), (1, 0.65)],
            }
            for point in paths[name]:
                click_point(*point)
        edits = sum(e["revisions"] for e in measurements)
        if name in ("guide", "point") and edits:
            placed_guide = vp.scene.guides[-1]
            if placed_guide.is_line != (name == "guide"):
                raise RuntimeError(f"{scenario}: wrong type of guide placed")
        tool.on_cancel(view.tools)
        if edits:
            record("undo", win._on_undo)
            record("redo", win._on_redo)
        print(scenario, edits, flush=True)
results.update(
    groups=n,
    placed_faces=n * 256,
    rss_peak_mb=rss_peak_mb(),
    panels_open=panels_open,
    visible_panels={
        type(p).__name__: not p.visibleRegion().isEmpty()
        for p in (
            win.tray.parts,
            win.tray.entity_info,
            win.scenes_dock.panel,
            win.layers_dock.panel,
            win.structure_dock,
            win._solid_inspector,
        )
    },
    samples=[
        {"phase": p, "stack": s, "count": c} for (p, s), c in samples.most_common(250)
    ],
)
Path(sys.argv[1]).write_text(json.dumps(results, indent=2) + "\n")
shutil.rmtree(folder, ignore_errors=True)
bad = [
    name
    for name, events in results.items()
    if "/" in name
    and (
        sum(e["revisions"] for e in events if e["event"] not in ("undo", "redo"))
        != (count if name.endswith(("/polyline", "/separate")) else 1)
        or any(e["revisions"] != 1 for e in events if e["event"] in ("undo", "redo"))
        or any(e["frame_ms"] is None for e in events if e["revisions"])
    )
]
if bad:
    print("Incomplete scenarios:", bad, flush=True)
os._exit(1 if bad else 0)
