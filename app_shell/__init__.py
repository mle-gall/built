"""The Python half of the ``built`` Rust app shell (rust/built_app).

The Rust binary owns the window, the event loop (winit), the menus and file
dialogs, and the wgpu renderer (linked in statically and registered as the
built-in ``ingetrazo_render`` module). This package is what it embeds: it
runs today's ``core/``, ``tools/`` and ``formats/`` unchanged, behind a
document controller and a view that are plain QObjects instead of Qt
widgets (see :mod:`views.view_host`).

Qt is still imported, headless (the ``offscreen`` platform), for the value
types the model and tools use (QVector3D, QMatrix4x4, QImage, QPainter for
overlay text, QSettings) and for QTimer. No widget is created and Qt's event
loop never runs: the Rust loop pumps posted events and timers once per
turn (:func:`app_shell.qt_host.pump`). Every Qt dependency the shell meets
is listed by ``scripts/qt_inventory.py`` and in docs/rust-app-shell.md.
"""
