"""Select the embedding host's preference store. Each handle has a private group stack."""
import os

_store = None
# A store opened without the shell choosing one: by BUILT_SETTINGS_DIR.
_implicit = (None, None)


def use_native(profile):
    from ingetrazo_core import NativeSettings
    global _store
    _store = NativeSettings(profile)


def settings():
    """The preferences: the store the shell selected (``use_native``), else
    the native store of ``BUILT_SETTINGS_DIR`` (the user's own without it),
    reopened when that variable changes."""
    global _implicit
    if _store is not None:
        return _store.fresh()
    profile = os.environ.get("BUILT_SETTINGS_DIR") or None
    known, store = _implicit
    if store is None or known != profile:
        from ingetrazo_core import NativeSettings
        store = NativeSettings(profile)
        _implicit = (profile, store)
    return store.fresh()
