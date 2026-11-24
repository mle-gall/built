"""Bind a host's pointer coordinates and input codes at the extension boundary."""
import sys

NATIVE = "built_shell" in sys.builtin_module_names

if NATIVE:
    from core.input_codes import Input as Qt, Type as QEvent, DeviceType
    from ingetrazo_core import (PixelPoint as QPoint, ScreenPoint as QPointF, ScreenRect as QRectF,
                                ScreenRect as QRect, ScreenSize as QSize)
else:
    from PySide6.QtCore import QEvent, QPoint, QPointF, QRect, QRectF, QSize, Qt
    from PySide6.QtGui import QInputDevice
    DeviceType = QInputDevice.DeviceType
