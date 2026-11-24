//! Canonical RGBA PNG for content-addressed tinted material textures. Older
//! documents use the libpng encoding's hash: retain its row filters, pHYs,
//! zlib strategy, window size and IDAT boundaries.
use pyo3::{prelude::*, types::PyBytes};

fn chunk(output: &mut Vec<u8>, tag: &[u8; 4], data: &[u8]) {
    output.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let begin = output.len();
    output.extend_from_slice(tag);
    output.extend_from_slice(data);
    let crc = crc32fast::hash(&output[begin..]);
    output.extend_from_slice(&crc.to_be_bytes());
}
fn paeth(left: u8, up: u8, corner: u8) -> u8 {
    let p = i32::from(left) + i32::from(up) - i32::from(corner);
    let (a, b, c) = (
        (p - i32::from(left)).abs(),
        (p - i32::from(up)).abs(),
        (p - i32::from(corner)).abs(),
    );
    if a <= b && a <= c {
        left
    } else if b <= c {
        up
    } else {
        corner
    }
}
fn filter(rows: &[u8], width: usize) -> Vec<u8> {
    let stride = 4 * width;
    let mut output = Vec::with_capacity(rows.len() + rows.len() / stride);
    let mut filters = [0, 1, 2, 3, 4].map(|_| vec![0; stride]);
    let mut previous = &[][..];
    for row in rows.chunks_exact(stride) {
        let mut scores = [0u64; 5];
        for (i, &c) in row.iter().enumerate() {
            let up = previous.get(i).copied().unwrap_or(0);
            let left = i.checked_sub(4).map_or(0, |j| row[j]);
            let corner = i
                .checked_sub(4)
                .and_then(|j| previous.get(j))
                .copied()
                .unwrap_or(0);
            let average = ((u16::from(left) + u16::from(up)) / 2) as u8;
            for (k, predictor) in [0, left, up, average, paeth(left, up, corner)]
                .into_iter()
                .enumerate()
            {
                let diff = c.wrapping_sub(predictor);
                filters[k][i] = diff;
                scores[k] += u64::from((diff as i8).unsigned_abs());
            }
        }
        let best = (0..5).min_by_key(|k| scores[*k]).unwrap();
        output.push(best as u8);
        output.extend_from_slice(&filters[best]);
        previous = row;
    }
    output
}
pub fn encode<'py>(
    py: Python<'py>,
    width: u32,
    height: u32,
    rgba: &[u8],
    source: &[u8],
) -> PyResult<Bound<'py, PyBytes>> {
    let filtered = py.detach(|| filter(rgba, width as usize));
    let window = filtered
        .len()
        .next_power_of_two()
        .trailing_zeros()
        .clamp(9, 15);
    // CPython's standard zlib uses the host deflater, as did the original writer.
    let encoder = py
        .import("zlib")?
        .call_method1("compressobj", (-1, 8, window, 8, 1))?;
    let mut compressed: Vec<u8> = encoder
        .call_method1("compress", (PyBytes::new(py, &filtered),))?
        .extract()?;
    compressed.extend_from_slice(encoder.call_method0("flush")?.cast::<PyBytes>()?.as_bytes());
    let mut output = b"\x89PNG\r\n\x1a\n".to_vec();
    let header = [
        &width.to_be_bytes()[..],
        &height.to_be_bytes(),
        &[8, 6, 0, 0, 0],
    ]
    .concat();
    chunk(&mut output, b"IHDR", &header);
    // Imported images can carry the logical display density of the host on
    // which the original tint was computed. Preserve it in the cache hash.
    let density = png::Decoder::new(std::io::Cursor::new(source))
        .read_info()
        .ok()
        .and_then(|r| r.info().pixel_dims)
        .map_or((3780, 3780, 1), |d| (d.xppu, d.yppu, d.unit as u8));
    let physical = [
        &density.0.to_be_bytes()[..],
        &density.1.to_be_bytes(),
        &[density.2],
    ]
    .concat();
    chunk(&mut output, b"pHYs", &physical);
    for part in compressed.chunks(8192) {
        chunk(&mut output, b"IDAT", part);
    }
    chunk(&mut output, b"IEND", &[]);
    Ok(PyBytes::new(py, &output))
}
