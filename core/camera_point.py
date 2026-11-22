# SPDX-License-Identifier: GPL-3.0-or-later
"""Double camera coordinates with a QVector3D projection for Qt APIs."""
import math
from PySide6.QtGui import QVector3D


class CameraPoint(QVector3D):
    """Qt may consume the binary32 base. Model arithmetic uses the doubles."""
    def __init__(self, x=0., y=None, z=None):
        if y is None and z is None:
            if hasattr(x, "toTuple"):
                x, y, z = x.toTuple()
            elif isinstance(x, (int, float)):
                x, y, z = x, 0., 0.
            else:
                x, y, z = x
        self._xyz = (float(x), float(y), float(z))
        super().__init__(*self._xyz)

    def x(self):
        return self._xyz[0]

    def y(self):
        return self._xyz[1]

    def z(self):
        return self._xyz[2]

    def toTuple(self):
        return self._xyz

    def __copy__(self):
        return CameraPoint(self)

    def __reduce__(self):
        return CameraPoint, self._xyz

    def __add__(self, other):
        return CameraPoint(tuple(a + b for a, b in zip(self._xyz, other.toTuple())))

    __radd__ = __add__
    __iadd__ = __add__

    def __sub__(self, other):
        return CameraPoint(tuple(a - b for a, b in zip(self._xyz, other.toTuple())))

    def __rsub__(self, other):
        return CameraPoint(tuple(a - b for a, b in zip(other.toTuple(), self._xyz)))

    __isub__ = __sub__

    def __mul__(self, k):
        if not isinstance(k, (float, int)):
            return NotImplemented
        return CameraPoint(tuple(v * k for v in self._xyz))

    __rmul__ = __mul__
    __imul__ = __mul__

    def __truediv__(self, k):
        return self * (1. / k)

    __itruediv__ = __truediv__

    def __neg__(self):
        return self * -1.

    def normalize(self):
        unit = self.normalized()
        self.setX(unit.x())
        self.setY(unit.y())
        self.setZ(unit.z())

    def length(self):
        return math.hypot(*self._xyz)

    def lengthSquared(self):
        return sum(v * v for v in self._xyz)

    def normalized(self):
        length = self.length()
        return self / length if length else CameraPoint()

    def _set(self, i, value):
        coords = list(self._xyz)
        coords[i] = float(value)
        self._xyz = tuple(coords)
        (super().setX, super().setY, super().setZ)[i](value)

    def setX(self, x):
        self._set(0, x)

    def setY(self, y):
        self._set(1, y)

    def setZ(self, z):
        self._set(2, z)
