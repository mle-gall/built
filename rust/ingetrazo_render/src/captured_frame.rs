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
        let mut encoder = png::Encoder::new(io::BufWriter::new(file), self.width, self.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(&self.rgba)?;
        writer.finish()?;
        Ok(())
    }
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
