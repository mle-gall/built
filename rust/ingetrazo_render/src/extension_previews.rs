//! Artwork for extension cards. Fetching goes through the installer's bounded
//! HTTPS client; file identity, decoding, scaling and retry policy live here.
use image::{imageops::FilterType, DynamicImage, ImageFormat, ImageReader, Limits};
use pyo3::{exceptions::PyValueError, prelude::*, types::PyBytes};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    hash::{DefaultHasher, Hash, Hasher},
    io::{Cursor, Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, LazyLock, Mutex,
    },
    time::{Duration, SystemTime},
};

pub const WIDTH: u32 = 256;
pub const HEIGHT: u32 = 160;
const LIMIT: usize = 8 << 20;
const REPO: &str = "https://raw.githubusercontent.com/ingelibre/ingetrazo-extensions/main";
const WEBSITE: &str = "https://ingetrazo.com/extensiones-img";
static SERIAL: AtomicU64 = AtomicU64::new(0);
type Pixels = Arc<[u8]>;
type MemoryKey = (PathBuf, SystemTime, u64);
static MEMORY: LazyLock<Mutex<HashMap<MemoryKey, Pixels>>> = LazyLock::new(Default::default);

fn read(path: &Path) -> Option<Vec<u8>> {
    let mut data = Vec::new();
    fs::File::open(path)
        .ok()?
        .take((LIMIT + 1) as u64)
        .read_to_end(&mut data)
        .ok()?;
    (data.len() <= LIMIT).then_some(data)
}

fn write(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().unwrap();
    fs::create_dir_all(parent)?;
    let tmp = parent.join(format!(
        ".preview-{}-{}",
        std::process::id(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = fs::File::create_new(&tmp)?;
    let result = file.write_all(data).and_then(|()| {
        drop(file);
        fs::rename(&tmp, path)
    });
    let _ = fs::remove_file(tmp);
    result
}

fn key(path: &Path) -> Option<MemoryKey> {
    let meta = path.metadata().ok()?;
    Some((path.into(), meta.modified().ok()?, meta.len()))
}

fn digest(value: impl Hash) -> String {
    let mut hash = DefaultHasher::new();
    value.hash(&mut hash);
    format!("{:016x}", hash.finish())
}

#[cfg(test)]
thread_local! {static DECODES: std::cell::Cell<usize> = const {std::cell::Cell::new(0)};}

fn decode(bytes: &[u8]) -> Option<DynamicImage> {
    #[cfg(test)]
    DECODES.with(|n| n.set(n.get() + 1));
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

#[pyclass(frozen, skip_from_py_object)]
#[derive(Clone, PartialEq, Eq)]
pub struct PreviewFile {
    #[pyo3(get)]
    pub icon: String,
    source: PathBuf,
    scaled_root: PathBuf,
    remote: Option<(String, bool, String)>,
    attempt: u8,
}

impl PreviewFile {
    pub fn from_entry(id: &str, entry: &Value, root: &Path) -> Option<Self> {
        let thumb = entry["thumb"].as_str();
        let shot = entry["screenshot"].as_str();
        let mut dir = root.to_owned();
        let (source, remote) = if entry["bundled"].is_string() {
            (PathBuf::from(shot?), None)
        } else {
            let name = thumb.or(shot)?;
            if name.contains(['/', '\\', '?', '#'])
                || Path::new(name).file_name()?.to_str()? != name
            {
                return None;
            }
            let path = Path::new(name);
            let stem = path.file_stem()?.to_str()?;
            let suffix = path.extension()?.to_str()?;
            if !["png", "jpg", "jpeg", "webp"].contains(&suffix.to_lowercase().as_str()) {
                return None;
            }
            if thumb.is_some() {
                dir.push("catalog-thumbs");
            }
            let print = entry["screenshot_sha"]
                .as_str()
                .filter(|s| s.len() == 16 && s.bytes().all(|c| c.is_ascii_hexdigit()))
                .unwrap_or_default();
            let source = dir.join(if print.is_empty() {
                name.into()
            } else {
                format!("{stem}-{print}.{suffix}")
            });
            (source, Some((name.into(), thumb.is_some(), print.into())))
        };
        // Bundled pictures can change without changing the catalogue file name.
        let identity = if remote.is_none() {
            digest(key(&source))
        } else {
            digest(&source)
        };
        Some(Self {
            icon: format!("ext:{id}:{identity}"),
            source,
            scaled_root: dir.join("scaled"),
            remote,
            attempt: 0,
        })
    }

    fn url(&self) -> Option<String> {
        let (name, thumb, print) = self.remote.as_ref()?;
        let name: String = name
            .bytes()
            .map(|c| {
                if c.is_ascii_alphanumeric() || b"-._~".contains(&c) {
                    (c as char).to_string()
                } else {
                    format!("%{c:02X}")
                }
            })
            .collect();
        Some(if self.attempt % 2 == 0 {
            format!(
                "{REPO}/{}/{name}",
                if *thumb { "thumbs" } else { "screenshots" }
            )
        } else if *thumb {
            format!("{WEBSITE}/thumbs/{name}")
        } else if print.is_empty() {
            format!("{WEBSITE}/{name}")
        } else {
            format!("{WEBSITE}/{name}?v={print}")
        })
    }

    fn download(&self, bytes: &[u8], obsolete: Vec<PathBuf>) -> Option<Pixels> {
        if bytes.len() > LIMIT {
            return None;
        }
        let image = decode(bytes)?;
        if write(&self.source, bytes).is_ok() {
            for path in obsolete {
                let _ = fs::remove_file(path);
            }
        }
        Some(self.store(image))
    }

    fn scaled(&self, k: &MemoryKey) -> PathBuf {
        self.scaled_root
            .join(format!("{}-{}.png", digest(&k.0), digest((k.1, k.2))))
    }

    fn cached(&self) -> Option<Pixels> {
        let k = key(&self.source)?;
        if let Some(rgba) = MEMORY.lock().ok()?.get(&k) {
            return Some(rgba.clone());
        }
        let image = read(&self.scaled(&k))
            .and_then(|d| decode(&d))
            .filter(|i| (i.width(), i.height()) == (WIDTH, HEIGHT));
        Some(match image {
            Some(image) => self.remember(k, image.into_rgba8().into_raw().into()),
            None => self.store(decode(&read(&self.source)?)?),
        })
    }
    fn remember(&self, k: MemoryKey, rgba: Pixels) -> Pixels {
        if let Ok(mut cache) = MEMORY.lock() {
            if cache.len() >= 128 {
                cache.clear()
            }
            cache.insert(k, rgba.clone());
        }
        rgba
    }
    fn store(&self, image: DynamicImage) -> Pixels {
        let image = image.resize_to_fill(WIDTH, HEIGHT, FilterType::Triangle);
        let rgba: Pixels = image.to_rgba8().into_raw().into();
        if let Some(k) = key(&self.source) {
            let mut bytes = Cursor::new(Vec::new());
            if image.write_to(&mut bytes, ImageFormat::Png).is_ok() {
                let _ = write(&self.scaled(&k), &bytes.into_inner());
            }
            return self.remember(k, rgba);
        }
        rgba
    }

    // Snapshot obsolete files before fetching. A later request for a new
    // version must not lose its newly downloaded file when this one completes.
    fn obsolete(&self) -> Vec<PathBuf> {
        let Some((name, _, print)) = &self.remote else {
            return vec![];
        };
        if print.is_empty() {
            return vec![];
        }
        let name = Path::new(name);
        let stem = name.file_stem().unwrap().to_string_lossy();
        let dir = self.source.parent().unwrap();
        let mut files = Vec::new();
        for file in fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = file.path();
            if path == self.source || path.extension() != name.extension() {
                continue;
            }
            let Some(s) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let old_print = s
                .strip_prefix(stem.as_ref())
                .and_then(|s| s.strip_prefix('-'));
            if s == stem
                || old_print
                    .is_some_and(|v| v.len() == 16 && v.bytes().all(|c| c.is_ascii_hexdigit()))
            {
                if let Some(k) = key(&path) {
                    files.push(self.scaled(&k));
                }
                files.push(path);
            }
        }
        files
    }
}

#[pymethods]
impl PreviewFile {
    /// Called by a host executor. Only the HTTPS request needs Python.
    fn run<'py>(
        &self,
        py: Python<'py>,
        fetch: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let cached = py.detach(|| self.cached());
        if let Some(rgba) = cached {
            return Ok(PyBytes::new(py, &rgba));
        }
        let url = self
            .url()
            .ok_or_else(|| PyValueError::new_err("Missing bundled preview"))?;
        let obsolete = py.detach(|| self.obsolete());
        let bytes = fetch.call1((url, LIMIT))?.cast_into::<PyBytes>()?;
        let bytes = bytes.as_bytes();
        let rgba = py
            .detach(|| self.download(bytes, obsolete))
            .ok_or_else(|| PyValueError::new_err("Invalid extension preview"))?;
        Ok(PyBytes::new(py, &rgba))
    }
}

#[derive(Default)]
struct Request {
    attempts: u8,
    busy: bool,
    next: Duration,
    ready: bool,
}

/// The renderer reports visible tiles first. Other previews fill idle worker
/// slots after the visible group and never hold up opening or scrolling.
#[derive(Default)]
pub struct Queue {
    files: BTreeMap<String, PreviewFile>,
    state: HashMap<String, Request>,
    visible: Vec<String>,
    pixels: BTreeMap<String, Py<PyBytes>>,
    pub revision: u64,
}

impl Queue {
    pub fn sync(&mut self, files: BTreeMap<String, PreviewFile>, py: Python<'_>) {
        self.state.retain(|id, _| files.contains_key(id));
        self.pixels.retain(|id, _| files.contains_key(id));
        for (id, file) in &files {
            if self.state.contains_key(id) {
                continue;
            }
            let memory = key(&file.source).and_then(|k| MEMORY.lock().ok()?.get(&k).cloned());
            if let Some(rgba) = memory {
                self.pixels
                    .insert(id.clone(), PyBytes::new(py, &rgba).unbind());
                self.state.insert(
                    id.clone(),
                    Request {
                        ready: true,
                        ..Default::default()
                    },
                );
            }
        }
        self.files = files;
    }
    pub fn visible(&mut self, ids: &[String]) {
        self.visible = ids.to_vec();
    }
    pub fn jobs(&mut self, now: Duration) -> Vec<PreviewFile> {
        if self.visible.is_empty() {
            return vec![];
        }
        let capacity = 3usize.saturating_sub(self.state.values().filter(|s| s.busy).count());
        let mut jobs = Vec::new();
        for id in self.visible.iter().chain(self.files.keys()) {
            if jobs.len() == capacity {
                break;
            }
            let Some(file) = self.files.get(id) else {
                continue;
            };
            let s = self.state.entry(id.clone()).or_default();
            if s.ready || s.busy || s.attempts >= 4 || now < s.next {
                continue;
            }
            let mut file = file.clone();
            file.attempt = s.attempts;
            s.busy = true;
            jobs.push(file);
        }
        jobs
    }
    pub fn done(&mut self, id: &str, pixels: Option<Py<PyBytes>>, now: Duration) {
        if let Some(s) = self.state.get_mut(id) {
            s.busy = false;
            s.ready = pixels.is_some();
            if let Some(data) = pixels {
                self.pixels.insert(id.to_owned(), data);
                self.revision += 1;
            }
            s.attempts += 1;
            s.next = now + Duration::from_secs(2 * u64::from(s.attempts));
        }
    }
    pub fn icons(&self, py: Python<'_>) -> Vec<(String, u32, u32, Py<PyBytes>)> {
        self.pixels
            .iter()
            .map(|(id, pixels)| (id.clone(), WIDTH, HEIGHT, pixels.clone_ref(py)))
            .collect()
    }
    pub fn pending(&self) -> bool {
        !self.visible.is_empty()
            && self.files.keys().any(|id| {
                self.state
                    .get(id)
                    .is_none_or(|s| !s.ready && s.attempts < 4)
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};
    use serde_json::json;
    fn png(size: (u32, u32), color: [u8; 4]) -> Vec<u8> {
        let image = DynamicImage::ImageRgba8(RgbaImage::from_pixel(size.0, size.1, Rgba(color)));
        let mut out = Cursor::new(Vec::new());
        image.write_to(&mut out, ImageFormat::Png).unwrap();
        out.into_inner()
    }
    fn temp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "built-preview-{name}-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ))
    }
    #[test]
    fn a_downloaded_picture_is_decoded_once() {
        let root = temp("decode");
        let file =
            PreviewFile::from_entry("demo", &json!({"thumb":"preview.webp"}), &root).unwrap();
        let data = png((512, 320), [255, 0, 0, 255]);
        DECODES.with(|n| n.set(0));
        let rgba = file.download(&data, vec![]).unwrap();
        assert_eq!(rgba.len(), (WIDTH * HEIGHT * 4) as usize);
        assert_eq!(DECODES.with(|n| n.get()), 1);
        assert_eq!(&*rgba, &*file.cached().unwrap());
        assert_eq!(DECODES.with(|n| n.get()), 1);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn scaled_screenshot_on_disk_expires_when_source_changes() {
        let root = temp("disk");
        let file =
            PreviewFile::from_entry("demo", &json!({"screenshot":"demo.png"}), &root).unwrap();
        let source = png((1024, 640), [255, 0, 0, 255]);
        file.download(&source, vec![]).unwrap();
        let k = key(&file.source).unwrap();
        let scaled = file.scaled(&k);
        let tiny = read(&scaled).unwrap();
        assert!(tiny.len() < source.len());
        assert_eq!(image::load_from_memory(&tiny).unwrap().width(), WIDTH);
        write(&scaled, &png((WIDTH, HEIGHT), [0, 0, 255, 255])).unwrap();
        MEMORY.lock().unwrap().remove(&k);
        assert_eq!(&file.cached().unwrap()[..4], &[0, 0, 255, 255]);
        write(&file.source, &png((160, 100), [0, 255, 0, 255])).unwrap();
        assert_eq!(&file.cached().unwrap()[..4], &[0, 255, 0, 255]);
        fs::remove_dir_all(root).unwrap();
    }
}
