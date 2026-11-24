"""Select the embedding host's preference store. Each handle has a private group stack."""
_store = None


def use_native(profile):
    from ingetrazo_core import NativeSettings
    global _store
    _store = NativeSettings(profile)


def settings():
    if _store is not None:
        return _store.fresh()
    # Classic and third-party standalone Qt entry points can configure their
    # own QSettings organization, test directory and format.
    from PySide6.QtCore import QSettings
    return QSettings()
