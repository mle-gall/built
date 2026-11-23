//! Registered fonts include user and application-activated fonts outside the system folders.
use core_foundation::{
    array::{CFArray, CFArrayRef},
    attributed_string::{CFMutableAttributedString, CFMutableAttributedStringRef},
    base::{CFRange, CFType, CFTypeRef, TCFType},
    string::{CFString, CFStringRef},
};

#[link(name = "CoreText", kind = "framework")]
extern "C" {
    fn CTFontManagerCopyAvailableFontFamilyNames() -> CFArrayRef;
    fn CTFontCreateWithName(name: CFStringRef, size: f64, matrix: *const ()) -> CFTypeRef;
    fn CTFontCreateCopyWithSymbolicTraits(
        font: CFTypeRef,
        size: f64,
        matrix: *const (),
        value: u32,
        mask: u32,
    ) -> CFTypeRef;
    fn CTLineCreateWithAttributedString(string: CFMutableAttributedStringRef) -> CFTypeRef;
    fn CTLineDraw(line: CFTypeRef, context: CFTypeRef);
    static kCTFontAttributeName: CFStringRef;
}

pub fn families() -> Vec<String> {
    // Copy functions return owned references, released by the CF wrappers.
    let raw = unsafe { CTFontManagerCopyAvailableFontFamilyNames() };
    if raw.is_null() {
        return Vec::new();
    }
    let array: CFArray<CFString> = unsafe { CFArray::wrap_under_create_rule(raw) };
    let mut names: Vec<_> = array.iter().map(|s| s.to_string()).collect();
    names.retain(|name| !name.starts_with('.'));
    names.sort_by_cached_key(|name| name.to_lowercase());
    names.dedup();
    names
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGColorSpaceCreateDeviceGray() -> CFTypeRef;
    fn CGBitmapContextCreate(
        data: *mut u8,
        width: usize,
        height: usize,
        bits: usize,
        stride: usize,
        space: CFTypeRef,
        info: u32,
    ) -> CFTypeRef;
    fn CGContextSetTextPosition(context: CFTypeRef, x: f64, y: f64);
}

/// Shape directly into a small coverage mask. CoreText handles collections,
/// variable faces, script shaping and glyph fallback without loading entire
/// system font files into the chrome atlas.
pub fn sample(
    name: &str,
    text: &str,
    bold: bool,
    italic: bool,
    size: f32,
    scale: f32,
    extent: egui::Vec2,
) -> Option<egui::FontImage> {
    let width = (extent.x * scale).ceil().clamp(1., 4096.) as usize;
    let height = (extent.y * scale).ceil().clamp(1., 512.) as usize;
    let mut pixels = vec![255u8; width * height];
    let size = f64::from(size * scale);
    let family = CFString::new(name);
    let sample = CFString::new(text);
    // Core Foundation's Create rule transfers one retain to each wrapper.
    // The bitmap context is dropped before the borrowed pixel buffer.
    unsafe {
        let own = |ptr: CFTypeRef| (!ptr.is_null()).then(|| CFType::wrap_under_create_rule(ptr));
        let base = own(CTFontCreateWithName(
            family.as_concrete_TypeRef(),
            size,
            std::ptr::null(),
        ))?;
        let traits = u32::from(italic) | (u32::from(bold) << 1);
        let font = own(CTFontCreateCopyWithSymbolicTraits(
            base.as_CFTypeRef(),
            size,
            std::ptr::null(),
            traits,
            3,
        ))
        .unwrap_or(base);
        let mut string = CFMutableAttributedString::new();
        string.replace_str(&sample, CFRange::init(0, 0));
        string.set_attribute(
            CFRange::init(0, string.char_len()),
            kCTFontAttributeName,
            &font,
        );
        let line = own(CTLineCreateWithAttributedString(
            string.as_concrete_TypeRef(),
        ))?;
        let gray = own(CGColorSpaceCreateDeviceGray())?;
        let ctx = own(CGBitmapContextCreate(
            pixels.as_mut_ptr(),
            width,
            height,
            8,
            width,
            gray.as_CFTypeRef(),
            0,
        ))?;
        CGContextSetTextPosition(
            ctx.as_CFTypeRef(),
            0.,
            (height as f64 - size) / 2. + size * 0.2,
        );
        CTLineDraw(line.as_CFTypeRef(), ctx.as_CFTypeRef());
    }
    Some(egui::FontImage {
        size: [width, height],
        pixels: pixels
            .into_iter()
            .map(|p| f32::from(255 - p) / 255.)
            .collect(),
    })
}

#[test]
fn installed_families_have_distinct_previews() {
    let names = families();
    assert!(names.len() > 1);
    let render = |name| {
        sample(
            name,
            "Built",
            false,
            false,
            super::tokens::TYPE_BODY,
            2.,
            egui::vec2(100., 24.),
        )
        .unwrap()
    };
    let a = render("Helvetica");
    assert!(a.pixels.iter().any(|&p| p > 0.));
    assert_ne!(a.pixels, render("Courier").pixels);
}
