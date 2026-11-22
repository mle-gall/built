"""Double coordinates for a Built point in the Python object mesh."""
from PySide6.QtGui import QVector3D

from core.mesh import Vertex


class BuiltVertex(Vertex):
    __slots__ = ("_source", "_projection")

    def __init__(self, point, *, key):
        super().__init__(QVector3D(*point), key=key)
        self._source = tuple(point)
        self._projection = self.position.toTuple()

    def position_tuple(self):
        # Qt tools can replace the vector or edit an individual component.
        # Preserve the source precision of each component they left alone.
        return tuple(v if p == c else c for v, p, c in
                     zip(self._source, self._projection, self.position.toTuple()))
