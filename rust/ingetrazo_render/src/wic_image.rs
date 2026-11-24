//! Read the first frame through the installed Windows Imaging Component codecs.
use super::{RgbaImage, MAX_PIXELS};
use std::{os::windows::ffi::OsStrExt, path::Path, ptr};
use windows::{
    core::PCWSTR,
    Win32::{
        Foundation::{GENERIC_READ, RPC_E_CHANGED_MODE},
        Graphics::Imaging::*,
        System::Com::*,
    },
};

struct Apartment(bool);
impl Drop for Apartment {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() }
        }
    }
}
struct Frame {
    bitmap: IWICBitmapFrameDecode,
    factory: IWICImagingFactory,
    _apartment: Apartment,
}
impl Frame {
    fn open(path: &Path) -> Option<Self> {
        let path: Vec<_> = path.as_os_str().encode_wide().chain([0]).collect();
        unsafe {
            let result = CoInitializeEx(None, COINIT_MULTITHREADED);
            if result.is_err() && result != RPC_E_CHANGED_MODE {
                return None;
            }
            let apartment = Apartment(result.is_ok());
            let factory: IWICImagingFactory =
                CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
            let decoder = factory
                .CreateDecoderFromFilename(
                    PCWSTR(path.as_ptr()),
                    None,
                    GENERIC_READ,
                    WICDecodeMetadataCacheOnDemand,
                )
                .ok()?;
            let bitmap = decoder.GetFrame(0).ok()?;
            Some(Self {
                bitmap,
                factory,
                _apartment: apartment,
            })
        }
    }
    fn size(&self) -> Option<(u32, u32)> {
        let (mut w, mut h) = (0, 0);
        unsafe {
            self.bitmap.GetSize(&mut w, &mut h).ok()?;
        }
        (w > 0 && h > 0).then_some((w, h))
    }
}
pub(super) fn header(path: &Path) -> Option<(u32, u32)> {
    Frame::open(path)?.size()
}
pub(super) fn read(path: &Path) -> Option<RgbaImage> {
    let frame = Frame::open(path)?;
    let (w, h) = frame.size()?;
    if u64::from(w) * u64::from(h) > MAX_PIXELS {
        return None;
    }
    unsafe {
        let convert = frame.factory.CreateFormatConverter().ok()?;
        convert
            .Initialize(
                &frame.bitmap,
                &GUID_WICPixelFormat32bppRGBA,
                WICBitmapDitherTypeNone,
                None,
                0.,
                WICBitmapPaletteTypeCustom,
            )
            .ok()?;
        let mut rgba = RgbaImage::new(w, h);
        convert.CopyPixels(ptr::null(), w * 4, &mut rgba).ok()?;
        Some(rgba)
    }
}
