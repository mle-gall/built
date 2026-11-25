//! Silent, constant-rate Matroska video. Each JPEG is independently decodable.
//! Cluster timecodes and the index use milliseconds, DefaultDuration uses ns.
use crate::atomic_document::AtomicDocument;
use image::{codecs::jpeg::JpegEncoder, ExtendedColorType};
use pyo3::{exceptions::PyValueError, prelude::*, pybacked::PyBackedBytes};
use std::{
    io::{self, Seek, SeekFrom, Write},
    path::PathBuf,
};

fn header(w: &mut impl Write, id: u32, size: usize) -> io::Result<()> {
    let id_bytes = id.to_be_bytes();
    w.write_all(&id_bytes[(id.leading_zeros() / 8) as usize..])?;
    // EBML reserves the all-ones length for an unknown size.
    let n = (1..=8)
        .find(|n| (size as u64) < (1u64 << (7 * n)) - 1)
        .ok_or_else(|| io::Error::other("Matroska element too large"))?;
    let length = ((size as u64) | (1u64 << (7 * n))).to_be_bytes();
    w.write_all(&length[8 - n..])
}
fn element(w: &mut impl Write, id: u32, data: &[u8]) -> io::Result<()> {
    header(w, id, data.len())?;
    w.write_all(data)
}
fn uint(w: &mut impl Write, id: u32, value: u64) -> io::Result<()> {
    let bytes = value.to_be_bytes();
    element(w, id, &bytes[(value.leading_zeros() / 8).min(7) as usize..])
}

#[pyclass(module = "built_core")]
pub struct MkvWriter {
    output: AtomicDocument,
    #[pyo3(get)]
    width: u32,
    #[pyo3(get)]
    height: u32,
    #[pyo3(get)]
    frames: u64,
    fps: u32,
    segment: u64,
    duration_offset: u64,
    cues: Vec<(u64, u64)>,
}

#[pymethods]
impl MkvWriter {
    #[new]
    fn new(path: PathBuf, width: u32, height: u32, fps: u32) -> PyResult<Self> {
        let width = width.max(2) & !1;
        let height = height.max(2) & !1;
        if width > 65534 || height > 65534 || fps == 0 || fps > 1000 {
            return Err(PyValueError::new_err(
                "Invalid video dimensions or frame rate",
            ));
        }
        let mut output = AtomicDocument::create(path)?;
        let file = output.output()?;
        let mut ebml = Vec::new();
        for (id, v) in [(0x4286, 1), (0x42f7, 1), (0x42f2, 4), (0x42f3, 8)] {
            uint(&mut ebml, id, v)?
        }
        element(&mut ebml, 0x4282, b"matroska")?;
        uint(&mut ebml, 0x4287, 4)?;
        uint(&mut ebml, 0x4285, 2)?;
        element(file, 0x1a45dfa3, &ebml)?;
        file.write_all(&[
            0x18, 0x53, 0x80, 0x67, 0x01, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        ])?;
        let segment = file.stream_position()?;
        let mut info = Vec::new();
        uint(&mut info, 0x2ad7b1, 1_000_000)?;
        element(&mut info, 0x4d80, b"Built")?;
        element(&mut info, 0x5741, b"Built")?;
        element(&mut info, 0x4489, &0f64.to_be_bytes())?;
        element(file, 0x1549a966, &info)?;
        let duration_offset = file.stream_position()? - 8;
        let mut video = Vec::new();
        uint(&mut video, 0xb0, width.into())?;
        uint(&mut video, 0xba, height.into())?;
        let mut track = Vec::new();
        uint(&mut track, 0xd7, 1)?;
        uint(&mut track, 0x73c5, 1)?;
        uint(&mut track, 0x83, 1)?;
        uint(
            &mut track,
            0x23e383,
            (1_000_000_000u64 + u64::from(fps / 2)) / u64::from(fps),
        )?;
        element(&mut track, 0x86, b"V_MJPEG")?;
        element(&mut track, 0xe0, &video)?;
        let mut tracks = Vec::new();
        element(&mut tracks, 0xae, &track)?;
        element(file, 0x1654ae6b, &tracks)?;
        Ok(Self {
            output,
            width,
            height,
            fps,
            frames: 0,
            segment,
            duration_offset,
            cues: Vec::new(),
        })
    }

    fn write(&mut self, py: Python<'_>, pixels: PyBackedBytes) -> PyResult<()> {
        if pixels.len() != self.width as usize * self.height as usize * 4 {
            return Err(PyValueError::new_err(
                "Expected one RGBA frame at the selected video size",
            ));
        }
        py.detach(|| -> io::Result<()> {
            let file = self.output.output()?;
            let mut rgb = Vec::with_capacity(pixels.len() / 4 * 3);
            for p in pixels.chunks_exact(4) {
                rgb.extend_from_slice(&p[..3])
            }
            // A SimpleBlock starts with the track, relative timestamp and keyframe flag.
            let mut block = vec![0x81, 0, 0, 0x80];
            JpegEncoder::new_with_quality(&mut block, 90)
                .encode(&rgb, self.width, self.height, ExtendedColorType::Rgb8)
                .map_err(io::Error::other)?;
            let timestamp = (self.frames * 1000 + u64::from(self.fps / 2)) / u64::from(self.fps);
            let mut prefix = Vec::new();
            uint(&mut prefix, 0xe7, timestamp)?;
            header(&mut prefix, 0xa3, block.len())?;
            if self.frames % self.fps as u64 == 0 {
                self.cues
                    .push((timestamp, file.stream_position()? - self.segment));
            }
            header(file, 0x1f43b675, prefix.len() + block.len())?;
            file.write_all(&prefix)?;
            file.write_all(&block)?;
            self.frames += 1;
            Ok(())
        })?;
        Ok(())
    }
    fn close(&mut self) -> PyResult<()> {
        if self.frames == 0 {
            return Err(PyValueError::new_err("A video needs at least one frame"));
        }
        let file = self.output.output()?;
        let mut cues = Vec::new();
        for (time, pos) in &self.cues {
            let mut position = Vec::new();
            uint(&mut position, 0xf7, 1)?;
            uint(&mut position, 0xf1, *pos)?;
            let mut point = Vec::new();
            uint(&mut point, 0xb3, *time)?;
            element(&mut point, 0xb7, &position)?;
            element(&mut cues, 0xbb, &point)?;
        }
        element(file, 0x1c53bb6b, &cues)?;
        file.seek(SeekFrom::Start(self.duration_offset))?;
        file.write_all(&(self.frames as f64 * 1000. / self.fps as f64).to_be_bytes())?;
        file.flush()?;
        self.output.commit()?;
        Ok(())
    }
    fn discard(&mut self) {
        self.output.discard()
    }
}

/// An explicit destination determines the container, even if encoder availability
/// changes after the save dialog. Never put fallback bytes in an MP4 file.
#[pyfunction]
pub fn video_writer(
    py: Python<'_>,
    path: PathBuf,
    width: u32,
    height: u32,
    fps: u32,
) -> PyResult<Py<PyAny>> {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("mp4") => Ok(Py::new(
            py,
            crate::mp4::Mp4Writer::new(py, path, width, height, fps)?,
        )?
        .into_any()),
        Some("mkv") => Ok(Py::new(py, MkvWriter::new(path, width, height, fps)?)?.into_any()),
        _ => Err(PyValueError::new_err(
            "Expected an .mp4 or .mkv destination",
        )),
    }
}
