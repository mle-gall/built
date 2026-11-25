//! First-page PDF imagery at 72 dpi, with its crop, rotation and transparency.
use super::{unpremultiply, MAX_PIXELS};
use core_foundation::{
    base::TCFType,
    url::{CFURLRef, CFURL},
};
use core_graphics::{
    base::{kCGBitmapByteOrder32Big, kCGImageAlphaPremultipliedLast},
    color_space::{kCGColorSpaceSRGB, CGColorSpace},
    context::CGContext,
    geometry::{CGAffineTransform, CGPoint, CGRect, CGSize},
};
use foreign_types::ForeignType;
use image::RgbaImage;
use std::{
    ffi::{c_int, c_void},
    fs::File,
    io::Read,
    path::Path,
    ptr::NonNull,
};

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGPDFDocumentCreateWithURL(url: CFURLRef) -> *mut c_void;
    fn CGPDFDocumentRelease(doc: *mut c_void);
    fn CGPDFDocumentGetPage(doc: *mut c_void, page: usize) -> *mut c_void;
    fn CGPDFPageGetBoxRect(page: *mut c_void, area: c_int) -> CGRect;
    fn CGPDFPageGetRotationAngle(page: *mut c_void) -> c_int;
    fn CGPDFPageGetDrawingTransform(
        page: *mut c_void,
        area: c_int,
        rect: CGRect,
        rotate: c_int,
        preserve: bool,
    ) -> CGAffineTransform;
    fn CGContextDrawPDFPage(context: *mut core_graphics::sys::CGContext, page: *mut c_void);
}

struct Document(NonNull<c_void>);
impl Drop for Document {
    fn drop(&mut self) {
        unsafe {
            CGPDFDocumentRelease(self.0.as_ptr());
        }
    }
}

pub(super) struct Page {
    _doc: Document,
    page: NonNull<c_void>,
    pub size: (u32, u32),
}
const CROP_BOX: c_int = 1;
impl Page {
    pub fn open(path: &Path) -> Option<Self> {
        let mut magic = [0; 5];
        File::open(path).ok()?.read_exact(&mut magic).ok()?;
        if &magic != b"%PDF-" {
            return None;
        }
        let url = CFURL::from_path(path, false)?;
        unsafe {
            let doc = Document(NonNull::new(CGPDFDocumentCreateWithURL(
                url.as_concrete_TypeRef(),
            ))?);
            let page = NonNull::new(CGPDFDocumentGetPage(doc.0.as_ptr(), 1))?;
            let crop = CGPDFPageGetBoxRect(page.as_ptr(), CROP_BOX).size;
            let (mut width, mut height) = (crop.width.round() as u32, crop.height.round() as u32);
            if CGPDFPageGetRotationAngle(page.as_ptr()).rem_euclid(180) == 90 {
                std::mem::swap(&mut width, &mut height);
            }
            if width == 0 || height == 0 {
                return None;
            }
            Some(Self {
                _doc: doc,
                page,
                size: (width, height),
            })
        }
    }
    pub fn read(self) -> Option<RgbaImage> {
        let (w, h) = self.size;
        if u64::from(w) * u64::from(h) > MAX_PIXELS {
            return None;
        }
        let mut out = RgbaImage::new(w, h);
        let space = CGColorSpace::create_with_name(unsafe { kCGColorSpaceSRGB })?;
        {
            let ctx = CGContext::create_bitmap_context(
                Some(out.as_mut_ptr().cast()),
                w as usize,
                h as usize,
                8,
                w as usize * 4,
                &space,
                kCGBitmapByteOrder32Big | kCGImageAlphaPremultipliedLast,
            );
            let bounds = CGRect::new(&CGPoint::new(0., 0.), &CGSize::new(w.into(), h.into()));
            unsafe {
                let matrix =
                    CGPDFPageGetDrawingTransform(self.page.as_ptr(), CROP_BOX, bounds, 0, true);
                ctx.concat_ctm(matrix);
                CGContextDrawPDFPage(ctx.as_ptr(), self.page.as_ptr());
            }
        }
        unpremultiply(&mut out);
        Some(out)
    }
}
