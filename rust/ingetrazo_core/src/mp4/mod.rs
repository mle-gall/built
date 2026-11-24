//! Stage movies in the destination directory. Platform muxers open files by
//! name and require a fresh path, so reserve a private directory for each one.
use pyo3::{exceptions::PyValueError, prelude::*, pybacked::PyBackedBytes};
use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::{mpsc, OnceLock},
    thread,
};

#[cfg(target_os = "macos")]
mod apple;
#[cfg(target_os = "macos")]
use apple::Encoder;
#[cfg(target_os = "windows")]
mod media_foundation;
#[cfg(target_os = "windows")]
use media_foundation::Encoder;
#[cfg(any(test, all(not(target_os = "macos"), not(target_os = "windows"))))]
mod system;
#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
use system::Encoder;

struct Stage(PathBuf);
impl Stage {
    fn beside(target: &Path) -> io::Result<Self> {
        use std::hash::{BuildHasher, RandomState};
        loop {
            let dir = target.with_file_name(format!(
                ".built-video-{:016x}",
                RandomState::new().hash_one(0)
            ));
            let mut builder = fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(&dir) {
                Ok(()) => return Ok(Self(dir)),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
    }
    fn movie(&self) -> PathBuf {
        self.0.join("video.mp4")
    }
}
impl Drop for Stage {
    fn drop(&mut self) {
        let _ = fs::remove_file(self.movie());
        let _ = fs::remove_dir(&self.0);
    }
}

fn probe() -> io::Result<()> {
    let stage = Stage::beside(&std::env::temp_dir().join("probe.mp4"))?;
    let mut encoder = Encoder::new(&stage.movie(), 64, 64, 30)?;
    encoder.frame(&[0; 64 * 64 * 4], 0)?;
    encoder.finish(1)
}

#[pyfunction]
pub fn video_format(py: Python<'_>) -> (&'static str, &'static str) {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    if py.detach(|| {
        *AVAILABLE.get_or_init(|| {
            // AVAssetWriter finalization and Media Foundation startup belong on a media thread.
            thread::Builder::new()
                .name("video probe".into())
                .spawn(probe)
                .is_ok_and(|worker| matches!(worker.join(), Ok(Ok(()))))
        })
    }) {
        (".mp4", "H.264 / MP4 (*.mp4)")
    } else {
        (".mkv", "Motion JPEG / Matroska (*.mkv)")
    }
}

enum Command {
    Frame(PyBackedBytes),
    Finish,
}
// Thread affinity matters for COM. The same worker opens, writes, finalizes
// and drops the encoder even when Python releases the last reference elsewhere.
#[pyclass(module = "ingetrazo_core")]
pub struct Mp4Writer {
    #[pyo3(get)]
    width: u32,
    #[pyo3(get)]
    height: u32,
    #[pyo3(get)]
    frames: u64,
    commands: Option<mpsc::Sender<Command>>,
    replies: std::sync::Mutex<mpsc::Receiver<io::Result<()>>>,
    worker: Option<thread::JoinHandle<()>>,
}

fn encode(
    target: &Path,
    width: u32,
    height: u32,
    fps: u32,
    commands: mpsc::Receiver<Command>,
    replies: &mpsc::Sender<io::Result<()>>,
) -> io::Result<()> {
    let stage = Stage::beside(target)?;
    let mut encoder = Encoder::new(&stage.movie(), width, height, fps)?;
    let mut frames = 0;
    let _ = replies.send(Ok(()));
    for command in commands {
        match command {
            Command::Frame(bytes) => {
                encoder.frame(&bytes, frames)?;
                frames += 1;
            }
            Command::Finish => {
                encoder.finish(frames)?;
                fs::rename(stage.movie(), target)?;
                break;
            }
        }
        let _ = replies.send(Ok(()));
    }
    Ok(())
}

impl Mp4Writer {
    fn reply(&mut self) -> io::Result<()> {
        self.replies
            .get_mut()
            .unwrap()
            .recv()
            .map_err(|_| io::Error::other("Video encoder stopped"))?
    }
    fn send(&mut self, command: Command) -> io::Result<()> {
        self.commands
            .as_ref()
            .ok_or_else(|| io::Error::other("Video writer is closed"))?
            .send(command)
            .map_err(|_| io::Error::other("Video encoder stopped"))?;
        self.reply()
    }
    fn stop(&mut self) {
        self.commands.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
impl Drop for Mp4Writer {
    fn drop(&mut self) {
        self.stop();
    }
}
#[pymethods]
impl Mp4Writer {
    #[new]
    pub(crate) fn new(
        py: Python<'_>,
        path: PathBuf,
        width: u32,
        height: u32,
        fps: u32,
    ) -> PyResult<Self> {
        let width = width.max(2) & !1;
        let height = height.max(2) & !1;
        if width > 65534 || height > 65534 || fps == 0 || fps > 1000 {
            return Err(PyValueError::new_err(
                "Invalid video dimensions or frame rate",
            ));
        }
        let (send, commands) = mpsc::channel();
        let (ack, replies) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("video export".into())
            .spawn(move || {
                let result = encode(&path, width, height, fps, commands, &ack);
                let _ = ack.send(result);
            })?;
        let mut writer = Self {
            width,
            height,
            frames: 0,
            commands: Some(send),
            replies: replies.into(),
            worker: Some(worker),
        };
        py.detach(|| writer.reply())?;
        Ok(writer)
    }
    fn write(&mut self, py: Python<'_>, pixels: PyBackedBytes) -> PyResult<()> {
        if pixels.len() != self.width as usize * self.height as usize * 4 {
            return Err(PyValueError::new_err(
                "Expected one RGBA frame at the selected video size",
            ));
        }
        py.detach(|| self.send(Command::Frame(pixels)))?;
        self.frames += 1;
        Ok(())
    }
    fn close(&mut self, py: Python<'_>) -> PyResult<()> {
        if self.frames == 0 {
            return Err(PyValueError::new_err("A video needs at least one frame"));
        }
        Ok(py.detach(|| {
            let result = self.send(Command::Finish);
            self.stop();
            result
        })?)
    }
    fn discard(&mut self, py: Python<'_>) {
        py.detach(|| self.stop());
    }
}
