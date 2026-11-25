"""A Built project in the native window (docs/builtproject-format.md §6).

Glue only: Rust's ``ProjectSession`` plans the open, keeps each member's
state, decides the editable file, writes the per-user session and gives
the Project tree and Problems their rows. This binds it to the window's
documents, the asynchronous open (views/document_open.py), cameras and
the read-only flag History checks.
"""
from __future__ import annotations

import json
from pathlib import Path

from core.i18n import tr


def session(win):
    return win.__dict__.get("_project")


def _touch(win) -> None:
    """The tree and problems show member states: rebuild them."""
    for doc in win._documents:
        view = getattr(doc, "stand_in", None)
        if view is not None:
            view._ui_tree_sig = None
            view.__dict__.pop("_ui_rules_cache", None)
    win._host.request_frame()


def open_project(win, path) -> bool:
    from ingetrazo_core import ProjectSession
    from core import recent
    from ingetrazo_render import chrome_dialogs as cd
    profile = getattr(win._host, "settings_dir", None)
    try:
        # The session lives in this profile (--settings, BUILT_SETTINGS_DIR),
        # else the platform's folder (docs/builtproject-format.md §3).
        s = ProjectSession(Path(path), Path(profile) if profile else None)
    except (OSError, ValueError) as exc:
        cd.notify(win, tr("Open project"), str(exc), tone="error")
        return False
    persist(win)
    win._project = s
    win._session_path = Path(s.path)
    conversion = s.conversion
    if conversion is not None and conversion[1] == "converted":
        cd.notify(win, tr("Project converted"), tr("{old} is now {new}.", old=Path(conversion[0]).name,
                                                    new=Path(s.path).name))
    recent.remember(s.path, replace=str(conversion[0]) if conversion else None)
    win._host.recent_changed()
    win._opening_session = True
    plan = s.plan()
    for abs_path, _role, camera, scene, exists in plan:
        if exists:
            _open_member(win, s, Path(abs_path), json.loads(camera) if camera else None, scene)
    if not s.pending():
        _finish(win, s)
    _touch(win)
    return True


def _open_member(win, s, path, camera, scene) -> None:
    from ingetrazo_render import shell_files as files
    from views import document_open
    doc = next((d for d in win._documents if d.same_file(path)), None)
    if doc is not None:
        _member_in(win, s, path, doc, None, scene)
        return
    try:
        ok = files.open_path(win, path)
    except Exception as exc:  # noqa: BLE001 — one member never stops the rest
        _member_failed(win, s, path, str(exc))
        return
    if document_open.pending(win, path):
        document_open.after_open(win, path, lambda d: _member_in(win, s, path, d, camera, scene),
                                 failed=lambda why: _member_failed(win, s, path, why))
        return
    doc = next((d for d in win._documents if d.same_file(path)), None)
    if ok and doc is not None:
        _member_in(win, s, path, doc, camera, scene)
    else:
        _member_failed(win, s, path, tr("could not be opened"))


def _member_in(win, s, path, doc, camera, scene) -> None:
    if session(win) is not s:
        return
    if camera and doc is win._doc:
        win._apply_camera_dict(camera)
    if scene and doc.viewport is not None:
        view = next((v for v in doc.viewport.scene.saved_views if v.name == scene), None)
        if view is not None and doc is win._doc:
            win._scene_tabs.mark_view(view)
    s.loaded(path)
    _settled(win, s)


def _member_failed(win, s, path, why) -> None:
    if session(win) is not s:
        return
    s.failed(path, str(why))
    _settled(win, s)


def _settled(win, s) -> None:
    refresh(win)
    if not s.pending():
        _finish(win, s)
    _touch(win)


def _finish(win, s) -> None:
    """Every member in or not: the session's order, the editable file in
    front, and one notice for what is missing."""
    if not win.__dict__.get("_opening_session"):
        return
    win._opening_session = False
    order = [p for p, *_ in s.plan()]
    rank = {p: i for i, p in enumerate(order)}

    def key(doc):
        p = str(Path(doc.path).resolve()) if doc.path else None
        return rank.get(p, len(order))
    win._documents.sort(key=key)
    editable = s.editable
    doc = next((d for d in win._documents if editable and d.same_file(editable)), None)
    if doc is not None and doc is not win._doc:
        win.activate_document(doc)
    refresh(win)
    problems = s.problems()
    if problems:
        from ingetrazo_render import chrome_dialogs as cd
        text = problems[0][1] if len(problems) == 1 else tr("{n} project files did not open", n=len(problems))
        cd.notify(win, tr("Project {name}", name=Path(s.path).stem), text, tone="warning")
    persist(win)
    win._update_title()


def refresh(win) -> None:
    """Only the editable file takes edits; other members are read-only
    (History and the agent bridge check ``worksession_read_only``)."""
    s = session(win)
    for doc in win._documents:
        vp = doc.viewport
        ro = bool(s is not None and doc.path is not None and s.is_member(doc.path) and s.read_only(doc.path))
        doc.read_only = ro
        if vp is not None:
            vp.scene.worksession_read_only = ro
    win._update_tab()


def persist(win) -> None:
    """This person's tabs, cameras, editable file and window layout."""
    s = session(win)
    # While the members come in, and once the window is closing (its tabs
    # are retired one by one: the session keeps them), nothing is written.
    if s is None or win.__dict__.get("_opening_session") or win._closed:
        return
    tabs = []
    marked = win._scene_tabs.marked_view()
    for doc in win._documents:
        if doc.path is None or doc.viewport is None:
            continue
        camera = json.dumps(win._camera_dict(doc.viewport))
        scene = getattr(marked, "name", None) if doc is win._doc else None
        tabs.append((Path(doc.path), camera, scene))
    view = getattr(win._doc, "stand_in", None)
    window = json.dumps({"lens": int(view.__dict__.get("_ui_lens", 0))}) if view is not None else None
    try:
        s.record(tabs, window)
    except ValueError as exc:       # an unwritable profile: say so, go on
        win.statusBar().showMessage(str(exc), 4000)


def set_active_file(win, path):
    s = session(win)
    if s is None:
        raise ValueError(tr("Open a project first"))
    path = Path(path).resolve()
    s.set_editable(path)
    if not any(d.same_file(path) for d in win._documents):
        _open_member(win, s, path, None, None)
    doc = next((d for d in win._documents if d.same_file(path)), None)
    if doc is not None:
        win.activate_document(doc)
    refresh(win)
    persist(win)
    _touch(win)
    return query(win)


def query(win):
    s = session(win)
    if s is None:
        return dict(project=None, active_file=None, files=[
            dict(path=str(Path(d.path).resolve()), role="host", read_only=False)
            for d in win._documents if d.path])
    return json.loads(s.query())


def closed(win, doc) -> None:
    s = session(win)
    if s is not None and doc.path is not None:
        s.closed(Path(doc.path))
        _touch(win)


def before_save(win, doc, path) -> None:
    """A new document saved while a project is open joins it as a host."""
    s = session(win)
    if s is None or doc.viewport is None:
        return
    from core import project
    if project.path_for(doc.viewport.scene) is None:
        project.attach(doc.viewport.scene, s.path, path)


def saved(win, doc) -> None:
    s = session(win)
    if s is None or doc.path is None:
        return
    joined = not s.is_member(doc.path)
    try:
        s.reload()
        s.loaded(Path(doc.path))
        if joined or s.editable is None:
            s.set_editable(Path(doc.path))
    except ValueError:
        return
    refresh(win)
    persist(win)
    _touch(win)


def tree_rows(win, query_text, expanded):
    s = session(win)
    return s.tree_rows(query_text, set(expanded)) if s is not None else []


def tree_action(win, verb, path) -> bool:
    """A project file row: open it; Edit makes it the editable file."""
    s = session(win)
    if s is None or not path:
        return False
    if verb == "open":
        doc = next((d for d in win._documents if d.same_file(path)), None)
        if doc is not None:
            win.activate_document(doc)
        elif Path(path).is_file():
            _open_member(win, s, Path(path), None, None)
        return True
    if verb == "edit":
        try:
            set_active_file(win, path)
        except ValueError as exc:
            win.statusBar().showMessage(str(exc), 4000)
        return True
    if verb == "locate":
        from views.filedialogs import file_dialogs
        found, _ = file_dialogs.getOpenFileName(win, tr("Locate {name}", name=Path(path).name), "",
                                                f"{Path(path).suffix[1:]} (*{Path(path).suffix})")
        if found:
            locate(win, path, found)
        return True
    return False


def locate(win, old, new) -> None:
    """A missing project file found elsewhere: the shared manifest names
    the new place (Rust rewrites its entry and links), then it opens."""
    from ingetrazo_core import project_relocate
    s = session(win)
    project_relocate(Path(s.path), Path(old), Path(new))
    s.reload()
    if Path(new).suffix.lower() in (".built", ".igz"):
        _open_member(win, s, Path(new).resolve(), None, None)
    persist(win)
    _touch(win)


def problems(win):
    """(severity, rule id, label, []) rows for the Problems list."""
    s = session(win)
    if s is None:
        return []
    return [("Needs attention", "project:" + p, text, []) for p, text in s.problems()]
