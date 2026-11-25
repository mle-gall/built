"""Double coordinates for a Built point in the Python object mesh."""
from functools import partialmethod
from ingetrazo_core import built_vertex_init, built_vertex_tuple

from core.mesh import Vertex


class BuiltVertex(Vertex):
    __slots__ = ("_source", "_projection")

    __init__ = partialmethod(built_vertex_init)
    position_tuple = partialmethod(built_vertex_tuple)
