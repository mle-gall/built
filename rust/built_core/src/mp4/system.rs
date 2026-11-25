//! Use an installed FFmpeg encoder. Test a real frame because a compiled-in
//! encoder can still lack its runtime library or hardware device.
use std::{
    ffi::OsStr,
    io::{self, Write},
    path::Path,
    process::{Child, Command, Stdio},
    sync::OnceLock,
};

pub(super) struct Encoder(Child);
impl Encoder {
    fn start(
        program: &OsStr,
        codec: &str,
        path: &Path,
        w: u32,
        h: u32,
        fps: u32,
    ) -> io::Result<Self> {
        Command::new(program)
            .args([
                "-nostdin",
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "rawvideo",
                "-pixel_format",
                "rgba",
                "-video_size",
                &format!("{w}x{h}"),
                "-framerate",
                &fps.to_string(),
                "-i",
                "pipe:0",
                "-an",
                "-c:v",
                codec,
                "-pix_fmt",
                "yuv420p",
                "-f",
                "mp4",
                "-y",
            ])
            .arg(path)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .map(Self)
    }
    fn discover(program: &OsStr) -> Option<&'static str> {
        let stage = super::Stage::beside(&std::env::temp_dir().join("probe.mp4")).ok()?;
        [
            "libx264",
            "libopenh264",
            "h264_v4l2m2m",
            "h264_nvenc",
            "h264_qsv",
        ]
        .into_iter()
        .find(|codec| {
            Self::start(program, codec, &stage.movie(), 64, 64, 30)
                .and_then(|mut encoder| {
                    encoder.frame(&[0; 64 * 64 * 4], 0)?;
                    encoder.finish(1)
                })
                .is_ok()
        })
    }
    pub(super) fn new(path: &Path, w: u32, h: u32, fps: u32) -> io::Result<Self> {
        static CODEC: OnceLock<Option<&str>> = OnceLock::new();
        let program = OsStr::new("ffmpeg");
        let codec = CODEC
            .get_or_init(|| Self::discover(program))
            .ok_or_else(|| io::Error::other("No system H.264 encoder available"))?;
        Self::start(program, codec, path, w, h, fps)
    }
    pub(super) fn frame(&mut self, bytes: &[u8], _: u64) -> io::Result<()> {
        self.0
            .stdin
            .as_mut()
            .ok_or_else(|| io::Error::other("Video encoder closed its input"))?
            .write_all(bytes)
    }
    pub(super) fn finish(mut self, _: u64) -> io::Result<()> {
        self.0.stdin.take();
        let status = self.0.wait()?;
        if !status.success() {
            return Err(io::Error::other(format!(
                "System video encoder failed ({status})"
            )));
        }
        Ok(())
    }
}
impl Drop for Encoder {
    fn drop(&mut self) {
        if matches!(self.0.try_wait(), Ok(None)) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn installed_encoder_writes_mp4() {
        if Encoder::discover(OsStr::new("ffmpeg")).is_none() {
            return;
        }
        let stage = super::super::Stage::beside(&std::env::temp_dir().join("check.mp4")).unwrap();
        let mut encoder = Encoder::new(&stage.movie(), 64, 64, 24).unwrap();
        encoder.frame(&[0; 64 * 64 * 4], 0).unwrap();
        encoder.finish(1).unwrap();
        assert_eq!(&std::fs::read(stage.movie()).unwrap()[4..8], b"ftyp");
    }
    #[test]
    fn missing_executable_never_advertises_mp4() {
        assert!(Encoder::discover(OsStr::new("/no/such/encoder")).is_none());
    }
}
