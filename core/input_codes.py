"""Named native input and presentation codes shared with the window's extension ABI."""
from enum import IntEnum, IntFlag
from types import SimpleNamespace


class Type(IntEnum):
    None_ = 0
    MouseButtonPress = 2
    MouseButtonRelease = 3
    MouseButtonDblClick = 4
    MouseMove = 5
    KeyPress = 6
    KeyRelease = 7
    Show = 17
    Wheel = 31
    ShortcutOverride = 51
    ChildAdded = 68
    ChildRemoved = 71
    ContextMenu = 82
    Shortcut = 117
    NativeGesture = 197


class DeviceType(IntFlag):
    Mouse = 1
    TouchScreen = 2
    TouchPad = 4


class StandardButton(IntFlag):
    NoButton = 0
    Ok = 0x400
    Save = 0x800
    SaveAll = 0x1000
    Open = 0x2000
    Yes = 0x4000
    YesToAll = 0x8000
    No = 0x10000
    NoToAll = 0x20000
    Abort = 0x40000
    Retry = 0x80000
    Ignore = 0x100000
    Close = 0x200000
    Cancel = 0x400000
    Discard = 0x800000
    Apply = 0x2000000
    Reset = 0x4000000
    RestoreDefaults = 0x8000000


class AlignmentFlag(IntFlag):
    AlignCenter = 132


class ApplicationAttribute(IntEnum):
    AA_DontCreateNativeWidgetSiblings = 4


class AspectRatioMode(IntEnum):
    IgnoreAspectRatio = 0


class BrushStyle(IntEnum):
    NoBrush = 0


class CursorShape(IntEnum):
    ArrowCursor = 0
    CrossCursor = 2
    WaitCursor = 3
    SizeVerCursor = 5
    SizeHorCursor = 6
    SizeBDiagCursor = 7
    SizeFDiagCursor = 8
    SizeAllCursor = 9
    BlankCursor = 10
    OpenHandCursor = 17
    ClosedHandCursor = 18
    BitmapCursor = 24


class DockWidgetArea(IntFlag):
    NoDockWidgetArea = 0
    LeftDockWidgetArea = 1
    RightDockWidgetArea = 2


class FocusPolicy(IntEnum):
    StrongFocus = 11


class FocusReason(IntEnum):
    OtherFocusReason = 7


class GlobalColor(IntEnum):
    transparent = 19


class Key(IntEnum):
    Key_Space = 32
    Key_0 = 48
    Key_1 = 49
    Key_2 = 50
    Key_3 = 51
    Key_4 = 52
    Key_5 = 53
    Key_6 = 54
    Key_7 = 55
    Key_8 = 56
    Key_9 = 57
    Key_A = 65
    Key_B = 66
    Key_C = 67
    Key_D = 68
    Key_E = 69
    Key_F = 70
    Key_G = 71
    Key_H = 72
    Key_I = 73
    Key_J = 74
    Key_K = 75
    Key_L = 76
    Key_M = 77
    Key_N = 78
    Key_O = 79
    Key_P = 80
    Key_Q = 81
    Key_R = 82
    Key_S = 83
    Key_T = 84
    Key_U = 85
    Key_V = 86
    Key_W = 87
    Key_X = 88
    Key_Y = 89
    Key_Z = 90
    Key_BracketLeft = 91
    Key_BracketRight = 93
    Key_Escape = 0x1000000
    Key_Tab = 0x1000001
    Key_Backtab = 0x1000002
    Key_Backspace = 0x1000003
    Key_Return = 0x1000004
    Key_Enter = 0x1000005
    Key_Delete = 0x1000007
    Key_Home = 0x1000010
    Key_End = 0x1000011
    Key_Left = 0x1000012
    Key_Up = 0x1000013
    Key_Right = 0x1000014
    Key_Down = 0x1000015
    Key_PageUp = 0x1000016
    Key_PageDown = 0x1000017
    Key_Shift = 0x1000020
    Key_Control = 0x1000021
    Key_Meta = 0x1000022
    Key_Alt = 0x1000023
    Key_F2 = 0x1000031
    Key_AltGr = 0x1001103


class KeyboardModifier(IntFlag):
    NoModifier = 0
    ShiftModifier = 0x2000000
    ControlModifier = 0x4000000
    AltModifier = 0x8000000
    MetaModifier = 0x10000000


class MouseButton(IntFlag):
    NoButton = 0
    LeftButton = 1
    RightButton = 2
    MiddleButton = 4


class MouseEventSource(IntEnum):
    MouseEventNotSynthesized = 0


class NativeGestureType(IntEnum):
    BeginNativeGesture = 0
    EndNativeGesture = 1
    PanNativeGesture = 2
    ZoomNativeGesture = 3
    SmartZoomNativeGesture = 4
    RotateNativeGesture = 5
    SwipeNativeGesture = 6


class PenCapStyle(IntEnum):
    FlatCap = 0
    RoundCap = 32


class PenJoinStyle(IntEnum):
    RoundJoin = 128


class PenStyle(IntEnum):
    NoPen = 0
    SolidLine = 1
    DashLine = 2
    DotLine = 3
    DashDotLine = 4
    DashDotDotLine = 5


class ScrollPhase(IntEnum):
    NoScrollPhase = 0
    ScrollBegin = 1
    ScrollUpdate = 2
    ScrollEnd = 3
    ScrollMomentum = 4


class TransformationMode(IntEnum):
    SmoothTransformation = 1


class WidgetAttribute(IntEnum):
    WA_PaintOnScreen = 8
    WA_NoSystemBackground = 9
    WA_SetCursor = 38
    WA_NativeWindow = 100


class WindowState(IntFlag):
    WindowNoState = 0
    WindowFullScreen = 4


def _codes(*groups):
    names = {key: value for group in groups for key, value in group.__members__.items()}
    names.update((group.__name__, group) for group in groups)
    return SimpleNamespace(**names)


Input = _codes(
    AlignmentFlag, ApplicationAttribute, AspectRatioMode, BrushStyle,
    CursorShape, DockWidgetArea, FocusPolicy, FocusReason,
    GlobalColor, Key, KeyboardModifier, MouseButton,
    MouseEventSource, NativeGestureType, PenCapStyle, PenJoinStyle,
    PenStyle, ScrollPhase, TransformationMode, WidgetAttribute,
    WindowState,
)
Input.KeyboardModifiers = KeyboardModifier
