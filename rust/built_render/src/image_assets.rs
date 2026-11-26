//! File imagery shared by material arrays, placed references and tool previews.
use image::{imageops, ImageError, ImageReader, RgbaImage};
use numpy::PyArray1;
use pyo3::{exceptions::PyValueError, prelude::*, types::PyBytes};
use resvg::{tiny_skia, usvg};
use std::{
    fs::File,
    io::{BufReader, Read},
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};

#[path = "bitmap_files.rs"]
mod bitmap_files;
#[cfg(target_os = "macos")]
#[path = "pdf_image.rs"]
mod pdf_image;

#[cfg(all(unix, any(not(target_os = "macos"), test)))]
#[path = "pixbuf_image.rs"]
mod desktop_pixbuf;
#[cfg(windows)]
#[path = "wic_image.rs"]
mod platform_image;
#[cfg(all(unix, not(target_os = "macos")))]
use desktop_pixbuf as platform_image;

const MAX_PIXELS: u64 = 64 * 1024 * 1024;
const MAX_SVG_BYTES: usize = 64 * 1024 * 1024;

fn reader(path: &Path) -> image::ImageResult<ImageReader<BufReader<File>>> {
    Ok(ImageReader::open(path)?.with_guessed_format()?)
}

fn svg_path(path: &Path) -> bool {
    path.extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.eq_ignore_ascii_case("svg") || s.eq_ignore_ascii_case("svgz"))
}

pub fn fonts() -> &'static Arc<usvg::fontdb::Database> {
    static FONTS: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    FONTS.get_or_init(|| {
        let mut db = usvg::fontdb::Database::new();
        db.load_system_fonts();
        share_interface_faces(&mut db);
        Arc::new(db)
    })
}

/// Map the interface families' files once. A face loaded from a file is
/// otherwise mapped again on every use, several times per drawn label:
/// that made each annotation `drawText` cost about a millisecond.
fn share_interface_faces(db: &mut usvg::fontdb::Database) {
    const FAMILIES: &[&str] = &["Helvetica Neue", "Helvetica", "Segoe UI", "DejaVu Sans", "Arial"];
    let mut ids: Vec<_> = db
        .faces()
        .filter(|f| f.families.iter().any(|(name, _)| FAMILIES.contains(&name.as_str())))
        .map(|f| f.id)
        .collect();
    let generic = db.family_name(&usvg::fontdb::Family::SansSerif).to_owned();
    ids.extend(db.faces().filter(|f| f.families.iter().any(|(name, _)| *name == generic)).map(|f| f.id));
    for id in ids {
        // SAFETY: system font files are not rewritten while the program runs;
        // fontdb maps them read-only, as it already does for each use.
        unsafe {
            db.make_shared_face_data(id);
        }
    }
}

pub fn svg_tree(input: &[u8], dir: Option<&Path>) -> Option<usvg::Tree> {
    let mut inflated = Vec::new();
    let input = if input.starts_with(&[0x1f, 0x8b]) {
        flate2::read::GzDecoder::new(input)
            .take(MAX_SVG_BYTES as u64 + 1)
            .read_to_end(&mut inflated)
            .ok()?;
        inflated.as_slice()
    } else {
        input
    };
    if input.len() > MAX_SVG_BYTES {
        return None;
    }
    let mut options = usvg::Options {
        fontdb: fonts().clone(),
        resources_dir: dir.map(Path::to_owned),
        ..Default::default()
    };
    if dir.is_none() {
        options.image_href_resolver.resolve_string = Box::new(|_, _| None);
    }
    usvg::Tree::from_str(std::str::from_utf8(input).ok()?, &options).ok()
}

fn file_svg(path: &Path) -> Option<usvg::Tree> {
    let mut data = Vec::new();
    File::open(path)
        .ok()?
        .take(MAX_SVG_BYTES as u64 + 1)
        .read_to_end(&mut data)
        .ok()?;
    svg_tree(&data, path.parent())
}

pub fn raster_svg(tree: &usvg::Tree, size: tiny_skia::IntSize, paper: bool) -> Option<RgbaImage> {
    if u64::from(size.width()) * u64::from(size.height()) > MAX_PIXELS {
        return None;
    }
    let mut pixmap = tiny_skia::Pixmap::new(size.width(), size.height())?;
    if paper {
        pixmap.fill(tiny_skia::Color::WHITE);
    }
    let transform = tiny_skia::Transform::from_scale(
        size.width() as f32 / tree.size().width(),
        size.height() as f32 / tree.size().height(),
    );
    resvg::render(tree, transform, &mut pixmap.as_mut());
    let mut result = RgbaImage::from_raw(size.width(), size.height(), pixmap.take())?;
    if !paper {
        unpremultiply(&mut result);
    }
    Some(result)
}

#[pyfunction]
pub fn svg_preview<'py>(py: Python<'py>, text: &str) -> PyResult<(u32, u32, Bound<'py, PyBytes>)> {
    let pixels = py
        .detach(|| {
            let tree = svg_tree(text.as_bytes(), None)?;
            let size = tree
                .size()
                .scale_to(tiny_skia::Size::from_wh(800., 800.)?)
                .to_int_size();
            raster_svg(&tree, size, true)
        })
        .ok_or_else(|| PyValueError::new_err("The exporter returned an empty SVG preview."))?;
    Ok((
        pixels.width(),
        pixels.height(),
        PyBytes::new(py, pixels.as_raw()),
    ))
}

pub fn unpremultiply(img: &mut RgbaImage) {
    for px in img.pixels_mut() {
        let alpha = u32::from(px[3]);
        if alpha != 0 && alpha != 255 {
            for c in &mut px.0[..3] {
                *c = ((u32::from(*c) * 255 + alpha / 2) / alpha).min(255) as u8;
            }
        }
    }
}

#[cfg(target_os = "macos")]
mod image_io {
    use super::*;
    use core_foundation::{
        base::{CFType, CFTypeRef, TCFType},
        dictionary::{CFDictionary, CFDictionaryRef},
        number::CFNumber,
        string::CFString,
        url::{CFURLRef, CFURL},
    };
    use core_graphics::{
        base::{kCGBitmapByteOrder32Big, kCGImageAlphaPremultipliedLast},
        color_space::{kCGColorSpaceSRGB, CGColorSpace},
        context::CGContext,
        geometry::{CGPoint, CGRect, CGSize},
        image::CGImage,
    };
    use foreign_types::ForeignType;
    use std::ptr;

    #[link(name = "ImageIO", kind = "framework")]
    extern "C" {
        fn CGImageSourceCreateWithURL(url: CFURLRef, opts: CFDictionaryRef) -> CFTypeRef;
        fn CGImageSourceCopyPropertiesAtIndex(
            src: CFTypeRef,
            index: usize,
            opts: CFDictionaryRef,
        ) -> CFDictionaryRef;
        fn CGImageSourceCreateImageAtIndex(
            src: CFTypeRef,
            index: usize,
            opts: CFDictionaryRef,
        ) -> *mut core_graphics::sys::CGImage;
    }

    fn source(path: &Path) -> Option<CFType> {
        let url = CFURL::from_path(path, false)?;
        // Each Create or Copy transfers one retain into its owning Rust wrapper.
        unsafe {
            let raw = CGImageSourceCreateWithURL(url.as_concrete_TypeRef(), ptr::null());
            (!raw.is_null()).then(|| CFType::wrap_under_create_rule(raw))
        }
    }

    fn dimensions(source: &CFType) -> Option<(u32, u32)> {
        let props: CFDictionary<CFString, CFType> = unsafe {
            let raw = CGImageSourceCopyPropertiesAtIndex(source.as_CFTypeRef(), 0, ptr::null());
            if raw.is_null() {
                return None;
            }
            CFDictionary::wrap_under_create_rule(raw)
        };
        let value = |key| {
            let n = props
                .find(&CFString::new(key))?
                .downcast::<CFNumber>()?
                .to_i64()?;
            u32::try_from(n).ok().filter(|&n| n > 0)
        };
        Some((value("PixelWidth")?, value("PixelHeight")?))
    }

    pub(super) fn header(path: &Path) -> Option<(u32, u32)> {
        dimensions(&source(path)?)
    }

    pub(super) fn read(path: &Path) -> Option<RgbaImage> {
        let source = source(path)?;
        let (w, h) = dimensions(&source)?;
        if u64::from(w) * u64::from(h) > MAX_PIXELS {
            return None;
        }
        let img = unsafe {
            let raw = CGImageSourceCreateImageAtIndex(source.as_CFTypeRef(), 0, ptr::null());
            if raw.is_null() {
                return None;
            }
            CGImage::from_ptr(raw)
        };
        let (w, h) = (
            u32::try_from(img.width()).ok()?,
            u32::try_from(img.height()).ok()?,
        );
        if w == 0 || h == 0 || u64::from(w) * u64::from(h) > MAX_PIXELS {
            return None;
        }
        let colors = CGColorSpace::create_with_name(unsafe { kCGColorSpaceSRGB })?;
        let mut pixels = RgbaImage::new(w, h);
        {
            let ctx = CGContext::create_bitmap_context(
                Some(pixels.as_mut_ptr().cast()),
                w as usize,
                h as usize,
                8,
                w as usize * 4,
                &colors,
                kCGImageAlphaPremultipliedLast | kCGBitmapByteOrder32Big,
            );
            ctx.draw_image(
                CGRect::new(&CGPoint::new(0., 0.), &CGSize::new(w.into(), h.into())),
                &img,
            );
        }
        unpremultiply(&mut pixels);
        Some(pixels)
    }
}

pub fn decode(py: Python<'_>, path: &Path) -> PyResult<Option<RgbaImage>> {
    Ok(py.detach(|| decode_file(path)))
}

pub(crate) fn decode_file(path: &Path) -> Option<RgbaImage> {
    let result = (|| -> image::ImageResult<Option<RgbaImage>> {
        if svg_path(path) {
            return Ok(
                file_svg(path).and_then(|tree| raster_svg(&tree, tree.size().to_int_size(), false))
            );
        }
        Ok(Some(reader(path)?.decode()?.into_rgba8()))
    })();
    match result {
        Ok(img) => img,
        Err(ImageError::Unsupported(_)) => {
            if let Some(img) = bitmap_files::read(path) {
                return Some(img);
            }
            #[cfg(target_os = "macos")]
            return {
                pdf_image::Page::open(path)
                    .and_then(|p| p.read())
                    .or_else(|| image_io::read(path))
            };
            #[cfg(any(windows, all(unix, not(target_os = "macos"))))]
            return platform_image::read(path);
            #[cfg(not(any(windows, unix)))]
            return None;
        }
        Err(_) => None,
    }
}

fn fit(pixels: RgbaImage, limit: u32) -> RgbaImage {
    let (w, h) = pixels.dimensions();
    let longest = w.max(h);
    if longest <= limit {
        return pixels;
    }
    let scaled = |n| ((u64::from(n) * u64::from(limit)) / u64::from(longest)).max(1) as u32;
    resize(pixels, scaled(w), scaled(h))
}

pub fn resize(mut pixels: RgbaImage, width: u32, height: u32) -> RgbaImage {
    if pixels.dimensions() == (width, height) {
        return pixels;
    }
    let translucent = pixels.pixels().any(|p| p[3] < 255);
    if translucent {
        premultiply(&mut pixels);
    }
    let mut pixels = imageops::resize(&pixels, width, height, imageops::FilterType::Triangle);
    if translucent {
        unpremultiply(&mut pixels);
    }
    pixels
}

pub fn premultiply(pixels: &mut RgbaImage) {
    for px in pixels.pixels_mut() {
        let alpha = u32::from(px[3]);
        for c in &mut px.0[..3] {
            *c = ((u32::from(*c) * alpha + 127) / 255) as u8;
        }
    }
}

pub fn texture(py: Python<'_>, path: &Path, limit: u32, flip: bool) -> PyResult<Option<RgbaImage>> {
    if limit == 0 {
        return Err(PyValueError::new_err("empty texture limit"));
    }
    Ok(decode(py, path)?.map(|pixels| {
        py.detach(|| {
            let mut pixels = fit(pixels, limit);
            if flip {
                imageops::flip_vertical_in_place(&mut pixels);
            }
            pixels
        })
    }))
}

type Texture<'py> = (u32, u32, Bound<'py, PyArray1<u8>>);
fn array(py: Python<'_>, pixels: RgbaImage) -> Texture<'_> {
    (
        pixels.width(),
        pixels.height(),
        PyArray1::from_vec(py, pixels.into_raw()),
    )
}
#[pyfunction]
pub fn reference_texture(
    py: Python<'_>,
    path: PathBuf,
    limit: u32,
) -> PyResult<Option<Texture<'_>>> {
    Ok(texture(py, &path, limit, false)?.map(|p| array(py, p)))
}

#[pyfunction]
pub fn fit_reference_rgba<'py>(
    py: Python<'py>,
    width: u32,
    height: u32,
    rgba: &[u8],
    limit: u32,
) -> PyResult<Texture<'py>> {
    if width == 0
        || height == 0
        || limit == 0
        || u64::from(width) * u64::from(height) != (rgba.len() / 4) as u64
        || rgba.len() % 4 != 0
    {
        return Err(PyValueError::new_err("invalid reference dimensions"));
    }
    let img = RgbaImage::from_raw(width, height, rgba.to_vec()).unwrap();
    Ok(array(py, py.detach(|| fit(img, limit))))
}

#[pyfunction]
pub fn raster_dimensions(py: Python<'_>, path: PathBuf) -> PyResult<Option<(u32, u32)>> {
    if svg_path(&path) {
        return Ok(py.detach(|| {
            file_svg(&path).map(|t| {
                let size = t.size().to_int_size();
                (size.width(), size.height())
            })
        }));
    }
    match py.detach(|| reader(&path)?.into_dimensions()) {
        Ok(size) => Ok(Some(size)),
        Err(ImageError::Unsupported(_)) => {
            if let Some(size) = py.detach(|| bitmap_files::read(&path).map(|p| p.dimensions())) {
                return Ok(Some(size));
            }
            #[cfg(target_os = "macos")]
            return Ok(py.detach(|| {
                pdf_image::Page::open(&path)
                    .map(|p| p.size)
                    .or_else(|| image_io::header(&path))
            }));
            #[cfg(any(windows, all(unix, not(target_os = "macos"))))]
            return Ok(py.detach(|| platform_image::header(&path)));
            #[cfg(not(any(windows, unix)))]
            return Ok(None);
        }
        Err(_) => Ok(None),
    }
}

#[pyfunction]
pub fn swatch_pixels<'py>(
    py: Python<'py>,
    path: PathBuf,
    side: u32,
    crop: bool,
) -> PyResult<Option<Bound<'py, PyBytes>>> {
    if side == 0 || side > 4096 {
        return Err(PyValueError::new_err("invalid swatch size"));
    }
    let Some(image) = decode(py, &path)? else {
        return Ok(None);
    };
    let pixels = py.detach(|| {
        let (w, h) = image.dimensions();
        let d = w.min(h);
        let source = if crop && w != h {
            imageops::crop_imm(&image, (w - d) / 2, (h - d) / 2, d, d).to_image()
        } else {
            image
        };
        resize(source, side, side)
    });
    Ok(Some(PyBytes::new(py, pixels.as_raw())))
}

pub(crate) fn billboard_image(py: Python<'_>, path: &Path) -> PyResult<Option<(u32, u32, bool)>> {
    match py.detach(|| reader(path)?.decode()) {
        Ok(image) => Ok(Some((
            image.width(),
            image.height(),
            image.color().has_alpha(),
        ))),
        Err(ImageError::Unsupported(_)) => Ok(decode(py, path)?.map(|image| {
            (
                image.width(),
                image.height(),
                image.pixels().any(|p| p[3] < 255),
            )
        })),
        Err(_) => Ok(None),
    }
}

#[pyfunction]
pub fn texture_has_alpha(py: Python<'_>, path: PathBuf) -> PyResult<bool> {
    use image::ImageDecoder;
    match py.detach(|| {
        reader(&path)?
            .into_decoder()
            .map(|d| d.color_type().has_alpha())
    }) {
        Ok(has) => Ok(has),
        Err(ImageError::Unsupported(_)) => {
            Ok(billboard_image(py, &path)?.is_some_and(|(_, _, a)| a))
        }
        Err(_) => Ok(false),
    }
}

/// Keep the legacy 32×32 nearest-neighbour test used by material import.
#[pyfunction]
pub fn texture_cutout(py: Python<'_>, path: PathBuf) -> PyResult<bool> {
    use image::ImageDecoder;
    let header = py.detach(|| reader(&path)?.into_decoder().map(|d| d.color_type()));
    if header.is_ok_and(|kind| !kind.has_alpha()) {
        return Ok(false);
    }
    Ok(decode(py, &path)?.is_some_and(|image| {
        let (w, h) = image.dimensions();
        (0..32)
            .any(|y| (0..32).any(|x| image.get_pixel(sample(x, w, 32), sample(y, h, 32))[3] < 32))
    }))
}

pub(crate) fn sample(i: u32, extent: u32, grid: u32) -> u32 {
    ((u64::from(2 * i + 1) * u64::from(extent)) / (2 * u64::from(grid))) as u32
}

#[pyfunction]
pub fn readable_tga(path: PathBuf) -> bool {
    File::open(path).is_ok_and(|f| {
        ImageReader::with_format(BufReader::new(f), image::ImageFormat::Tga)
            .into_dimensions()
            .is_ok()
    })
}

#[cfg(all(test, any(windows, unix)))]
mod platform_codec_tests {
    #[cfg(unix)]
    use super::desktop_pixbuf as codec;
    #[cfg(windows)]
    use super::platform_image as codec;
    use super::*;

    #[test]
    fn installed_desktop_codec_reads_a_real_frame_and_its_header() {
        #[cfg(unix)]
        if !codec::installed() {
            return; // Minimal/headless installs can run without GdkPixbuf.
        }
        let directory = std::env::temp_dir();
        let path = directory.join(format!("built-codec-frame-{}.custom", std::process::id()));
        let expected = RgbaImage::from_raw(1, 2, vec![32, 80, 128, 128, 220, 16, 50, 255]).unwrap();
        expected
            .save_with_format(&path, image::ImageFormat::Png)
            .unwrap();
        let header = codec::header(&path);
        let rgba = codec::read(&path);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(header, Some((1, 2)));
        assert_eq!(rgba, Some(expected));
        assert!(codec::read(&path).is_none());
    }
}
