//! Linux desktop codec modules, when GdkPixbuf is installed. Keep it optional
//! at load time so headless and minimal installs retain the built-in codecs.
use super::{RgbaImage, MAX_PIXELS};
use libloading::Library;
use std::{
    ffi::{c_char, c_void, CString},
    os::unix::ffi::OsStrExt,
    path::Path,
    ptr::{self, NonNull},
    sync::OnceLock,
};

type Handle = *mut c_void;
type Property = unsafe extern "C" fn(Handle) -> i32;
struct Api {
    _library: Library,
    info: unsafe extern "C" fn(*const c_char, *mut i32, *mut i32) -> Handle,
    load: unsafe extern "C" fn(*const c_char, i32, i32, i32, *mut Handle) -> Handle,
    width: Property,
    height: Property,
    bits: Property,
    channels: Property,
    stride: Property,
    length: unsafe extern "C" fn(Handle) -> usize,
    pixels: unsafe extern "C" fn(Handle) -> *const u8,
    unref: unsafe extern "C" fn(Handle),
}
impl Api {
    unsafe fn load() -> Option<Self> {
        #[cfg(target_os = "macos")]
        let name = "libgdk_pixbuf-2.0.0.dylib";
        #[cfg(not(target_os = "macos"))]
        let name = "libgdk_pixbuf-2.0.so.0";
        let library = Library::new(name).ok()?;
        Some(Self {
            info: *library.get(b"gdk_pixbuf_get_file_info\0").ok()?,
            load: *library.get(b"gdk_pixbuf_new_from_file_at_scale\0").ok()?,
            width: *library.get(b"gdk_pixbuf_get_width\0").ok()?,
            height: *library.get(b"gdk_pixbuf_get_height\0").ok()?,
            bits: *library.get(b"gdk_pixbuf_get_bits_per_sample\0").ok()?,
            channels: *library.get(b"gdk_pixbuf_get_n_channels\0").ok()?,
            stride: *library.get(b"gdk_pixbuf_get_rowstride\0").ok()?,
            length: *library.get(b"gdk_pixbuf_get_byte_length\0").ok()?,
            pixels: *library.get(b"gdk_pixbuf_read_pixels\0").ok()?,
            unref: *library.get(b"g_object_unref\0").ok()?,
            _library: library,
        })
    }
    fn get() -> Option<&'static Self> {
        static API: OnceLock<Option<Api>> = OnceLock::new();
        API.get_or_init(|| unsafe { Self::load() }).as_ref()
    }
    fn dimensions(&self, path: &CString) -> Option<(u32, u32)> {
        let (mut w, mut h) = (0, 0);
        let format = unsafe { (self.info)(path.as_ptr(), &mut w, &mut h) };
        (!format.is_null() && w > 0 && h > 0).then_some((w as u32, h as u32))
    }
}
struct Pixbuf(NonNull<c_void>, &'static Api);
impl Drop for Pixbuf {
    fn drop(&mut self) {
        unsafe {
            (self.1.unref)(self.0.as_ptr());
        }
    }
}

pub(super) fn header(path: &Path) -> Option<(u32, u32)> {
    Api::get()?.dimensions(&CString::new(path.as_os_str().as_bytes()).ok()?)
}
pub(super) fn read(path: &Path) -> Option<RgbaImage> {
    let api = Api::get()?;
    let path = CString::new(path.as_os_str().as_bytes()).ok()?;
    let (w, h) = api.dimensions(&path)?;
    if u64::from(w) * u64::from(h) > MAX_PIXELS {
        return None;
    }
    unsafe {
        let raw = (api.load)(path.as_ptr(), w as i32, h as i32, 0, ptr::null_mut());
        let _owned = Pixbuf(NonNull::new(raw)?, api);
        if (api.width)(raw) != w as i32 || (api.height)(raw) != h as i32 || (api.bits)(raw) != 8 {
            return None;
        }
        let pixels = NonNull::new((api.pixels)(raw).cast_mut())?;
        let bytes = std::slice::from_raw_parts(pixels.as_ptr(), (api.length)(raw));
        rgba_rows(
            w,
            h,
            usize::try_from((api.channels)(raw)).ok()?,
            usize::try_from((api.stride)(raw)).ok()?,
            bytes,
        )
    }
}
fn rgba_rows(w: u32, h: u32, channels: usize, stride: usize, bytes: &[u8]) -> Option<RgbaImage> {
    if !matches!(channels, 3 | 4) || w == 0 || h == 0 {
        return None;
    }
    let width = (w as usize).checked_mul(channels)?;
    if stride < width || bytes.len() < stride.checked_mul(h as usize - 1)?.checked_add(width)? {
        return None;
    }
    let mut output = RgbaImage::new(w, h);
    for (input, row) in bytes
        .chunks(stride)
        .zip(output.chunks_exact_mut(w as usize * 4))
    {
        if channels == 4 {
            row.copy_from_slice(&input[..width]);
        } else {
            for (src, dst) in input[..width].chunks_exact(3).zip(row.chunks_exact_mut(4)) {
                dst.copy_from_slice(&[src[0], src[1], src[2], 255]);
            }
        }
    }
    Some(output)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn padding_is_optional_after_last_row() {
        let data = [1, 2, 3, 91, 4, 5, 6];
        assert_eq!(
            rgba_rows(1, 2, 3, 4, &data).unwrap().as_raw(),
            &[1, 2, 3, 255, 4, 5, 6, 255]
        );
        assert!(rgba_rows(1, 2, 3, 4, &data[..6]).is_none());
        assert_eq!(
            rgba_rows(1, 1, 4, 4, &data[..4]).unwrap().as_raw(),
            &[1, 2, 3, 91]
        );
    }
}

#[cfg(test)]
pub(super) fn installed() -> bool {
    Api::get().is_some()
}
