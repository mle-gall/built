"""Window input values shared by the native event loop, scripts and plugins."""
from ingetrazo_core import DeviceType, Input, PixelPoint, ScreenPoint, ScreenRect, ScreenSize, Type

NATIVE = True

# Retain the original extension imports over the native value types.
QEvent = Type
Qt = Input
QPoint = PixelPoint
QPointF = ScreenPoint
QRect = QRectF = ScreenRect
QSize = ScreenSize
