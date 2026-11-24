//! Pixels and physical dimensions from the same GPU readback.
use std::{fs, io, path::Path};

pub struct CapturedFrame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl CapturedFrame {
    pub fn write_png(&self, path: &Path) -> io::Result<()> {
        let size = (self.width as usize)
            .checked_mul(self.height as usize)
            .and_then(|n| n.checked_mul(4));
        if size != Some(self.rgba.len()) || self.rgba.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid capture dimensions",
            ));
        }
        if let Some(dir) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(dir)?;
        }
        let file = fs::File::create(path)?;
        encode_png(
            io::BufWriter::new(file),
            self.width,
            self.height,
            &self.rgba,
        )?;
        Ok(())
    }
}

fn encode_png(
    dest: impl io::Write,
    width: u32,
    height: u32,
    pixels: &[u8],
) -> Result<(), png::EncodingError> {
    let mut enc = png::Encoder::new(dest, width, height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header()?;
    writer.write_image_data(pixels)?;
    writer.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_keeps_target_size_and_straight_alpha() {
        let dir = std::env::temp_dir().join(format!("built-frame-{}", std::process::id()));
        let path = dir.join("nested/frame.png");
        let frame = CapturedFrame {
            width: 1,
            height: 2,
            rgba: vec![1, 2, 3, 5, 8, 13, 21, 255],
        };
        frame.write_png(&path).unwrap();
        let image = image::open(&path).unwrap().into_rgba8();
        assert_eq!(image.dimensions(), (1, 2));
        assert_eq!(image.as_raw(), &frame.rgba);
        let invalid = CapturedFrame { width: 2, ..frame };
        assert!(invalid.write_png(&path).is_err());
        assert_eq!(image, image::open(&path).unwrap().into_rgba8());
        fs::remove_dir_all(dir).unwrap();
    }
}

/// Discard background-only archive thumbnails, sampling at pixel centres and
/// quantizing RGB to tolerate dithering and JPEG noise in older documents.
#[pyo3::pyfunction]
pub fn blank_picture(w: u32, h: u32, rgba: &[u8]) -> bool {
    if w == 0
        || h == 0
        || (w as usize)
            .checked_mul(h as usize)
            .and_then(|n| n.checked_mul(4))
            != Some(rgba.len())
    {
        return true;
    }
    let mut histogram = [0u16; 32 * 32 * 32];
    for y in 0..64 {
        let row = crate::image_assets::sample(y, h, 64) as usize * w as usize;
        for x in 0..64 {
            let at = (row + crate::image_assets::sample(x, w, 64) as usize) * 4;
            let bin = |v: u8| usize::from(v >> 3);
            let count =
                &mut histogram[bin(rgba[at]) * 1024 + bin(rgba[at + 1]) * 32 + bin(rgba[at + 2])];
            *count += 1;
            if *count >= 4035 {
                return true;
            } // ceil(0.985 * 64 * 64)
        }
    }
    false
}

#[pyo3::pyfunction]
pub fn archive_picture_png<'py>(
    py: pyo3::Python<'py>,
    w: u32,
    h: u32,
    rgba: &[u8],
) -> pyo3::PyResult<Option<pyo3::Bound<'py, pyo3::types::PyBytes>>> {
    if blank_picture(w, h, rgba) {
        return Ok(None);
    }
    let mut data = Vec::new();
    encode_png(&mut data, w, h, rgba)
        .map_err(|e| pyo3::exceptions::PyIOError::new_err(e.to_string()))?;
    Ok(Some(pyo3::types::PyBytes::new(py, &data)))
}

#[pyo3::pyfunction]
pub fn archive_thumbnail<'py>(
    py: pyo3::Python<'py>,
    png: &[u8],
    width: u32,
) -> Option<(u32, u32, pyo3::Bound<'py, pyo3::types::PyBytes>)> {
    if !png.starts_with(b"\x89PNG\r\n\x1a\n") || png.len() > 4 * 1024 * 1024 || width == 0 {
        return None;
    }
    let image = py.detach(|| {
        let image = crate::extension_previews::decode(png)?.into_rgba8();
        let (w, h) = image.dimensions();
        if blank_picture(w, h, image.as_raw()) {
            return None;
        }
        if w <= width {
            return Some(image);
        }
        let height = (u64::from(h) * u64::from(width) + u64::from(w) / 2) / u64::from(w);
        Some(crate::image_assets::resize(
            image,
            width,
            (height as u32).max(1),
        ))
    })?;
    Some((
        image.width(),
        image.height(),
        pyo3::types::PyBytes::new(py, image.as_raw()),
    ))
}
