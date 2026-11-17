"""Bounded, synthetic linked-document tool probe; never opens user files.
Run with the project's Python: scripts/composite_tools_probe.py OUT.json.
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
os.environ['INGETRAZO_WGPU'] = '1'
os.environ['INGETRAZO_RUST'] = '1'
from scripts.open_path_probe import rss_now_mb, rss_peak_mb
from PySide6.QtCore import QSettings, Qt, QPointF, QEvent
from PySide6.QtGui import QVector3D, QMatrix4x4, QMouseEvent
from PySide6.QtWidgets import QApplication
folder = Path(tempfile.mkdtemp(prefix='composite-tools-probe-'))
QSettings.setDefaultFormat(QSettings.IniFormat)
for scope in (QSettings.UserScope, QSettings.SystemScope):
    QSettings.setPath(QSettings.IniFormat, scope, str(folder))
from core import autosave
autosave.autosave_dir = lambda: folder
autosave.pending = lambda *a, **k: None
main_thread = threading.get_ident()
samples = Counter()
phase = ''
def watch():
    start = time.monotonic()
    next_rss = 0
    while True:
        now = time.monotonic()
        if now > next_rss:
            rss = rss_now_mb()
            if (rss and rss > 5500) or now-start > 240:
                print('ABORT: RSS/time budget', rss, flush=True)
                os._exit(2)
            next_rss = now + .5
        if phase:
            f = sys._current_frames().get(main_thread)
            stack = []
            while f:
                stack.append(f'{Path(f.f_code.co_filename).name}:{f.f_code.co_name}:{f.f_lineno}')
                f = f.f_back
            samples[(phase, ' <- '.join(stack[:8]))] += 1
        time.sleep(.002)
threading.Thread(target=watch, daemon=True).start()
app = QApplication([])
from core.scene import Scene
from core.mesh import Mesh
from core.group import Group
from core import links
from formats import igz
n = int(os.environ.get('PROBE_GROUPS', '20000'))
source = Scene()
mesh = Mesh()
for i in range(100):
    x, y = i % 10, i // 10
    mesh.add_face([QVector3D(x,y,0), QVector3D(x+1,y,0), QVector3D(x+1,y+1,0), QVector3D(x,y+1,0)])
for i in range(n):
    mat = QMatrix4x4()
    mat.translate(100 + (i%200)*12, 100 + (i//200)*12, 0)
    g = Group(mesh, name=f'generated-{i}')
    g.xform = mat
    source.groups.append(g)
igz.save_scene(source, folder/'source.igz')
print('source saved', rss_now_mb(), flush=True)
host = Scene()
host.groups.append(links.make_link(host, folder/'source.igz'))
small = Group(name='editable')
small.component = False
small.mesh.add_face([QVector3D(0,0,0), QVector3D(1,0,0), QVector3D(1,1,0), QVector3D(0,1,0)])
host.groups.append(small)
igz.save_scene(host, folder/'host.igz')
del source, host
from views.main_window import MainWindow
win = MainWindow(defer_extensions=True)
win.resize(1100, 750)
win.setWindowFlag(Qt.WindowStaysOnTopHint, True)
win.setAttribute(Qt.WA_ShowWithoutActivating, True)
win.show()
win.open_path(folder/'host.igz')
vp = win.viewport
view = win._doc.stand_in
win._leave_empty_area()
win._doc_stack.setCurrentWidget(view)
view.show()
def settle(seconds):
    end = time.monotonic()+seconds
    while time.monotonic() < end:
        app.processEvents()
        time.sleep(.005)
settle(3)
small = next(g for g in vp.scene.groups if g.name == 'editable')
vp.begin_group_edit(small)
view.set_scene(vp.scene)
view.camera.target = QVector3D(.5,.5,0)
view.camera.distance = 8
view.update()
settle(3)
results = {}
frames = []
view.frameReady.connect(lambda *a: frames.append(time.perf_counter()))
for key in ('line', 'protractor', 'eraser', 'tape'):
    win._activate_tool(key)
    settle(.5)
    measurements = []
    revision = vp.history.revision
    for j, (kind, dx) in enumerate([('move',0), ('press',0), ('release',0), ('move',60), ('press',60), ('release',60), ('move',90), ('press',90), ('release',90)]):
        point = QPointF(view.width()/2+dx, view.height()/2)
        typ = {'move':QEvent.MouseMove,'press':QEvent.MouseButtonPress,'release':QEvent.MouseButtonRelease}[kind]
        button = Qt.NoButton if kind == 'move' else Qt.LeftButton
        buttons = Qt.LeftButton if kind == 'press' else Qt.NoButton
        event = QMouseEvent(typ, point, point, button, buttons, Qt.NoModifier)
        phase = key+'/'+kind
        frames.clear()
        old_revision = vp.history.revision
        t = time.perf_counter()
        getattr(view, {'move':'mouseMoveEvent','press':'mousePressEvent','release':'mouseReleaseEvent'}[kind])(event)
        elapsed = (time.perf_counter()-t)*1000
        measurements.append({'handler':kind,'ms':elapsed})
        phase = key+'/frame'
        settle(.06)
        if vp.history.revision != old_revision:
            measurements[-1]['edit_to_frame_ms'] = (frames[0]-t)*1000 if frames else None
        phase = ''
    results[key] = {'events':measurements, 'edits':vp.history.revision-revision}
    print(key, results[key], flush=True)
    view.tools.tool.on_cancel(view.tools)
# A known committed edit, measured through presentation, independent of pick location.
from core.history import MoveVerticesCommand
phase = 'commit-to-frame'
frames.clear()
t = time.perf_counter()
vp.history.execute(MoveVerticesCommand([v.position for v in vp.scene.mesh.vertices], QVector3D(0,0,.01)))
vp.notify_scene_changed()
while not frames and time.perf_counter()-t < 5:
    app.processEvents()
results['commit_to_frame_ms'] = (frames[0]-t)*1000 if frames else None
phase = ''
results.update(groups=n, placed_faces=n*100, rss_peak_mb=rss_peak_mb(), samples=[{'phase':p,'stack':s,'count':c} for (p,s),c in samples.most_common(150)])
Path(sys.argv[1]).write_text(json.dumps(results, indent=2)+'\n')
print('RESULT', results['commit_to_frame_ms'], results['rss_peak_mb'], flush=True)
shutil.rmtree(folder, ignore_errors=True)
os._exit(0)
