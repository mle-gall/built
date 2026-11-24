"""Select native annotation styles or the widget host's painter values."""
from core.input_types import NATIVE

if NATIVE:
    from ingetrazo_render import AnnotationFont as QFont, PaintColor as QColor, PaintPen as QPen
    QPolygonF = list
else:
    from PySide6.QtGui import QColor, QFont, QPen, QPolygonF
