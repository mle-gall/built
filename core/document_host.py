"""Per-document state and the window bound to that document's sheet composer."""
from __future__ import annotations

from pathlib import Path
from typing import Optional

from core.i18n import tr


class Document:
    """One open project: its viewport and the window's record of it."""

    view_layout = "single"

    def __init__(self, viewport=None) -> None:
        self.viewport = viewport
        #: The .igz it was opened from or saved to (None = never saved).
        self.path: Optional[Path] = None
        #: The imported file's name, shown until the model is saved as .igz.
        self.import_name: Optional[str] = None
        #: Content version at the last save (see MainWindow._saved_version).
        self.saved_content: int = 0
        #: Content version the auto-save last wrote, or None.
        self.autosaved_version: Optional[int] = None
        #: Which untitled auto-save slot it writes while it has no path
        #: (``core.autosave.slot_for``): 1 is the classic ``untitled.igz``.
        self.untitled_no: int = 1
        #: The sheet composer of this document, created on demand.
        self.composer = None
        #: The composer was on screen when the user left this tab.
        self.composer_shown = False
        self.last_sheet: int = 0
        #: Its viewport's frame, small, shown on hover over its tab; drawn
        #: then, if it is due (MainWindow._tab_picture).
        self.thumbnail = None
        self.picture_due = False
        #: Its scenes shown as tabs under the viewport (views/scene_tabs.py);
        #: its tab's right-click menu turns them off and on.
        self.scene_tabs: bool = True
        #: How many views it shows (views/multi_view.py LAYOUTS) and the
        #: grid holding them, made the first time it shows more than one.
        self.view_grid = None
        #: What an extension put in this project's place in the viewport
        #: stack with ``setCentralWidget`` (MultiView's grid of views, which
        #: holds this viewport), or a blank placeholder between its
        #: ``takeCentralWidget`` and that call; None = the viewport itself.
        #: See MainWindow.centralWidget.
        self.stand_in = None
        #: The view an extension made ``window.viewport`` while this project
        #: is in front (a split pane), or None. ``viewport`` stays the
        #: project's own: what is saved comes from there (#1106).
        self.front_view = None

    def slot_widget(self):
        """The widget this project shows in the viewport stack."""
        grid = getattr(self, "view_grid", None)
        # Under Metal the stand-in (the native view) sits in a cell of the
        # project's grid: the grid is what the stack shows.
        if (grid is not None and self.stand_in is not None
                and getattr(grid, "display", None) is self.stand_in):
            return grid
        if self.stand_in is not None:
            return self.stand_in
        return getattr(self.viewport, "keys_stand_in", None) or self.viewport

    def display_name(self) -> str:
        if self.path is not None:
            return Path(self.path).name
        if self.import_name:
            return str(self.import_name)
        return tr("Untitled")

    def same_file(self, path) -> bool:
        """Whether ``path`` names the file this document holds."""
        if self.path is None or path is None:
            return False
        try:
            return Path(self.path).resolve() == Path(path).resolve()
        except OSError:
            return Path(self.path).absolute() == Path(path).absolute()


class DocumentHost:
    """The main window as one document's sheet composer sees it: everything
    is the window's, except the viewport and the file, which stay that
    document's even while another tab is in front — a composer's timer must
    never render the other project into its frames."""

    def __init__(self, window, doc: Document) -> None:
        object.__setattr__(self, "_host_window", window)
        object.__setattr__(self, "_host_doc", doc)

    @property
    def viewport(self):
        return self._host_doc.viewport

    @property
    def _current_path(self):
        return self._host_doc.path

    def __getattr__(self, name):
        return getattr(self._host_window, name)

    def __setattr__(self, name, value) -> None:
        setattr(self._host_window, name, value)
