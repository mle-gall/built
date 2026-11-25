use image::{DynamicImage, ImageReader, Limits};
use std::{fs, io::{Cursor, Read}, path::Path};
const LIMIT: usize = 8 << 20;
pub(crate) fn read(path: &Path) -> Option<Vec<u8>> {
    let mut data = Vec::new();
    fs::File::open(path)
        .ok()?
        .take((LIMIT + 1) as u64)
        .read_to_end(&mut data)
        .ok()?;
    (data.len() <= LIMIT).then_some(data)
}

pub(crate) fn decode(bytes: &[u8]) -> Option<DynamicImage> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(16384);
    limits.max_image_height = Some(16384);
    limits.max_alloc = Some(128 << 20);
    reader.limits(limits);
    reader.decode().ok()
}
