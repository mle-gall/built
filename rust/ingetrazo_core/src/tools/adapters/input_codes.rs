// SPDX-License-Identifier: GPL-3.0-or-later
//! Stable event codes used by the winit adapter and the Python plugin API.
use super::*;

const GROUPS: &[(&str, bool, &[(&str, u32)])] = &[
    (
        "Type",
        false,
        &[
            ("None_", 0x0),
            ("MouseButtonPress", 0x2),
            ("MouseButtonRelease", 0x3),
            ("MouseButtonDblClick", 0x4),
            ("MouseMove", 0x5),
            ("KeyPress", 0x6),
            ("KeyRelease", 0x7),
            ("Show", 0x11),
            ("Wheel", 0x1f),
            ("ShortcutOverride", 0x33),
            ("ChildAdded", 0x44),
            ("ChildRemoved", 0x47),
            ("ContextMenu", 0x52),
            ("Shortcut", 0x75),
            ("NativeGesture", 0xc5),
        ],
    ),
    (
        "DeviceType",
        true,
        &[("Mouse", 0x1), ("TouchScreen", 0x2), ("TouchPad", 0x4)],
    ),
    (
        "StandardButton",
        true,
        &[
            ("NoButton", 0x0),
            ("Ok", 0x400),
            ("Save", 0x800),
            ("SaveAll", 0x1000),
            ("Open", 0x2000),
            ("Yes", 0x4000),
            ("YesToAll", 0x8000),
            ("No", 0x10000),
            ("NoToAll", 0x20000),
            ("Abort", 0x40000),
            ("Retry", 0x80000),
            ("Ignore", 0x100000),
            ("Close", 0x200000),
            ("Cancel", 0x400000),
            ("Discard", 0x800000),
            ("Apply", 0x2000000),
            ("Reset", 0x4000000),
            ("RestoreDefaults", 0x8000000),
        ],
    ),
    ("AlignmentFlag", true, &[("AlignCenter", 0x84)]),
    (
        "ApplicationAttribute",
        false,
        &[("AA_DontCreateNativeWidgetSiblings", 0x4)],
    ),
    ("AspectRatioMode", false, &[("IgnoreAspectRatio", 0x0)]),
    ("BrushStyle", false, &[("NoBrush", 0x0)]),
    (
        "CursorShape",
        false,
        &[
            ("ArrowCursor", 0x0),
            ("CrossCursor", 0x2),
            ("WaitCursor", 0x3),
            ("SizeVerCursor", 0x5),
            ("SizeHorCursor", 0x6),
            ("SizeBDiagCursor", 0x7),
            ("SizeFDiagCursor", 0x8),
            ("SizeAllCursor", 0x9),
            ("BlankCursor", 0xa),
            ("OpenHandCursor", 0x11),
            ("ClosedHandCursor", 0x12),
            ("BitmapCursor", 0x18),
        ],
    ),
    (
        "DockWidgetArea",
        true,
        &[
            ("NoDockWidgetArea", 0x0),
            ("LeftDockWidgetArea", 0x1),
            ("RightDockWidgetArea", 0x2),
        ],
    ),
    ("FocusPolicy", false, &[("StrongFocus", 0xb)]),
    ("FocusReason", false, &[("OtherFocusReason", 0x7)]),
    ("GlobalColor", false, &[("transparent", 0x13)]),
    (
        "Key",
        false,
        &[
            ("Key_Space", 0x20),
            ("Key_0", 0x30),
            ("Key_1", 0x31),
            ("Key_2", 0x32),
            ("Key_3", 0x33),
            ("Key_4", 0x34),
            ("Key_5", 0x35),
            ("Key_6", 0x36),
            ("Key_7", 0x37),
            ("Key_8", 0x38),
            ("Key_9", 0x39),
            ("Key_A", 0x41),
            ("Key_B", 0x42),
            ("Key_C", 0x43),
            ("Key_D", 0x44),
            ("Key_E", 0x45),
            ("Key_F", 0x46),
            ("Key_G", 0x47),
            ("Key_H", 0x48),
            ("Key_I", 0x49),
            ("Key_J", 0x4a),
            ("Key_K", 0x4b),
            ("Key_L", 0x4c),
            ("Key_M", 0x4d),
            ("Key_N", 0x4e),
            ("Key_O", 0x4f),
            ("Key_P", 0x50),
            ("Key_Q", 0x51),
            ("Key_R", 0x52),
            ("Key_S", 0x53),
            ("Key_T", 0x54),
            ("Key_U", 0x55),
            ("Key_V", 0x56),
            ("Key_W", 0x57),
            ("Key_X", 0x58),
            ("Key_Y", 0x59),
            ("Key_Z", 0x5a),
            ("Key_BracketLeft", 0x5b),
            ("Key_BracketRight", 0x5d),
            ("Key_Escape", 0x1000000),
            ("Key_Tab", 0x1000001),
            ("Key_Backtab", 0x1000002),
            ("Key_Backspace", 0x1000003),
            ("Key_Return", 0x1000004),
            ("Key_Enter", 0x1000005),
            ("Key_Delete", 0x1000007),
            ("Key_Home", 0x1000010),
            ("Key_End", 0x1000011),
            ("Key_Left", 0x1000012),
            ("Key_Up", 0x1000013),
            ("Key_Right", 0x1000014),
            ("Key_Down", 0x1000015),
            ("Key_PageUp", 0x1000016),
            ("Key_PageDown", 0x1000017),
            ("Key_Shift", 0x1000020),
            ("Key_Control", 0x1000021),
            ("Key_Meta", 0x1000022),
            ("Key_Alt", 0x1000023),
            ("Key_F2", 0x1000031),
            ("Key_AltGr", 0x1001103),
        ],
    ),
    (
        "KeyboardModifier",
        true,
        &[
            ("NoModifier", 0x0),
            ("ShiftModifier", 0x2000000),
            ("ControlModifier", 0x4000000),
            ("AltModifier", 0x8000000),
            ("MetaModifier", 0x10000000),
        ],
    ),
    (
        "MouseButton",
        true,
        &[
            ("NoButton", 0x0),
            ("LeftButton", 0x1),
            ("RightButton", 0x2),
            ("MiddleButton", 0x4),
        ],
    ),
    (
        "MouseEventSource",
        false,
        &[("MouseEventNotSynthesized", 0x0)],
    ),
    (
        "NativeGestureType",
        false,
        &[
            ("BeginNativeGesture", 0x0),
            ("EndNativeGesture", 0x1),
            ("PanNativeGesture", 0x2),
            ("ZoomNativeGesture", 0x3),
            ("SmartZoomNativeGesture", 0x4),
            ("RotateNativeGesture", 0x5),
            ("SwipeNativeGesture", 0x6),
        ],
    ),
    (
        "PenCapStyle",
        false,
        &[("FlatCap", 0x0), ("RoundCap", 0x20)],
    ),
    ("PenJoinStyle", false, &[("RoundJoin", 0x80)]),
    (
        "PenStyle",
        false,
        &[
            ("NoPen", 0x0),
            ("SolidLine", 0x1),
            ("DashLine", 0x2),
            ("DotLine", 0x3),
            ("DashDotLine", 0x4),
            ("DashDotDotLine", 0x5),
        ],
    ),
    (
        "ScrollPhase",
        false,
        &[
            ("NoScrollPhase", 0x0),
            ("ScrollBegin", 0x1),
            ("ScrollUpdate", 0x2),
            ("ScrollEnd", 0x3),
            ("ScrollMomentum", 0x4),
        ],
    ),
    (
        "TransformationMode",
        false,
        &[("SmoothTransformation", 0x1)],
    ),
    (
        "WidgetAttribute",
        false,
        &[
            ("WA_PaintOnScreen", 0x8),
            ("WA_NoSystemBackground", 0x9),
            ("WA_SetCursor", 0x26),
            ("WA_NativeWindow", 0x64),
        ],
    ),
    (
        "WindowState",
        true,
        &[("WindowNoState", 0x0), ("WindowFullScreen", 0x4)],
    ),
];

pub(super) fn register(module: &Bound<PyModule>) -> PyResult<()> {
    let py = module.py();
    let enums = py.import("enum")?;
    let input = py.import("types")?.call_method0("SimpleNamespace")?;
    for &(name, flags, members) in GROUPS {
        let values = PyDict::new(py);
        for &(label, code) in members {
            values.set_item(label, code)?;
        }
        let namespace = PyDict::new(py);
        namespace.set_item("module", "ingetrazo_core")?;
        let ty = enums.call_method(
            if flags { "IntFlag" } else { "IntEnum" },
            (name, &values),
            Some(&namespace),
        )?;
        module.add(name, &ty)?;
        if !matches!(name, "Type" | "DeviceType" | "StandardButton") {
            input.setattr(name, &ty)?;
            for &(label, _) in members {
                input.setattr(label, ty.getattr(label)?)?;
            }
        }
    }
    input.setattr("KeyboardModifiers", module.getattr("KeyboardModifier")?)?;
    module.add("Input", input)
}
