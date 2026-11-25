// SPDX-License-Identifier: GPL-3.0-or-later
//! Bounded CPU neighborhoods of a settled, per-view R32Uint ID texture.
const SIDE: u32 = 512;
pub const MAX_BYTES: u32 = SIDE * 4 * SIDE;
const MAX_TILES: usize = 4;

#[derive(Clone, Copy)]
pub struct Region {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Region {
    pub fn around(x: u32, y: u32, width: u32, height: u32) -> Self {
        // Center the first read on the pointer. Crossing a fixed grid boundary
        // a pixel later should not immediately wait for the next GPU submission.
        let w = width.min(SIDE);
        let h = height.min(SIDE);
        Self {
            x: x.saturating_sub(w / 2).min(width - w),
            y: y.saturating_sub(h / 2).min(height - h),
            width: w,
            height: h,
        }
    }

    pub fn stride(self) -> u32 {
        (self.width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
    }

    pub fn bytes(self) -> u32 {
        self.height * self.stride()
    }

    fn contains(self, x: u32, y: u32) -> bool {
        (self.x..self.x + self.width).contains(&x) && (self.y..self.y + self.height).contains(&y)
    }

    pub fn row(self, bytes: &[u8], x: u32, y: u32) -> i64 {
        let at = ((y - self.y) * self.stride() + (x - self.x) * 4) as usize;
        i64::from(u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())) - 1
    }
}

pub struct Tiles(Vec<(Region, Vec<u8>)>);

impl Tiles {
    pub const EMPTY: Self = Self(Vec::new());

    pub fn get(&self, x: u32, y: u32) -> Option<i64> {
        let (region, bytes) = self
            .0
            .iter()
            .rev()
            .find(|(region, _)| region.contains(x, y))?;
        Some(region.row(bytes, x, y))
    }

    pub fn insert(&mut self, region: Region, bytes: Vec<u8>) {
        if self.0.len() == MAX_TILES {
            self.0.remove(0);
        }
        self.0.push((region, bytes));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipped_view_uses_padded_rows_and_its_last_texel() {
        let region = Region::around(70, 518, 71, 519);
        assert_eq!(
            (
                region.x,
                region.y,
                region.width,
                region.height,
                region.stride()
            ),
            (0, 7, 71, 512, 512)
        );
        let mut bytes = vec![0; region.bytes() as usize];
        let offset = 511 * 512 + 70 * 4;
        bytes[offset..offset + 4].copy_from_slice(&9u32.to_le_bytes());
        let mut tiles = Tiles::EMPTY;
        tiles.insert(region, bytes);
        assert_eq!(tiles.get(70, 518), Some(8));
        assert_eq!(tiles.get(69, 518), Some(-1));
        assert_eq!(tiles.get(70, 6), None);
    }
}
