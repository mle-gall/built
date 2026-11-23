# SPDX-License-Identifier: GPL-3.0-or-later
"""Narrow binary64 model values only when uploading a Qt shader uniform."""
from ingetrazo_core import Matrix4, Vec3, Vec4
from PySide6.QtGui import QMatrix4x4, QVector3D, QVector4D
from PySide6.QtOpenGL import QOpenGLShaderProgram


class ShaderProgram(QOpenGLShaderProgram):
    def setUniformValue(self, location, value):
        if isinstance(value, Matrix4):
            value = QMatrix4x4(value.copyDataTo())
        elif isinstance(value, Vec3):
            value = QVector3D(*value.toTuple())
        elif isinstance(value, Vec4):
            value = QVector4D(*value.toTuple())
        return super().setUniformValue(location, value)
