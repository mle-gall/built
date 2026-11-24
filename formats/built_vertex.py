"""Double coordinates for a Built point in the Python object mesh."""
from functools import partialmethod
from ingetrazo_core import Vec3 as QVector3D, built_vertex_tuple

from core.mesh import Vertex


class BuiltVertex(Vertex):
    __slots__ = ("_source", "_projection")

    def __init__(self, point, *, key):
        super().__init__(QVector3D(*point), key=key)
        self._source = tuple(point)
        self._projection = self.position.toTuple()

    position_tuple = partialmethod(built_vertex_tuple)
