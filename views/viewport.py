"""Compatibility imports for extensions written against the original document API."""
from views import document_interaction
from views.document_controller import DocumentController as Viewport


def __getattr__(name):
    return getattr(document_interaction, name)
