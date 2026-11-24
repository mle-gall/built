//! Media Foundation is an optional Windows feature. Load its factories from
//! System32 on demand so the core module also loads on Windows N without it.
use libloading::os::windows::{Library, LOAD_LIBRARY_SEARCH_SYSTEM32};
use std::{ffi::c_void, io, os::windows::ffi::OsStrExt, path::Path, ptr};
use windows::{
    core::{Interface, GUID, HRESULT, PCWSTR},
    Win32::{
        Foundation::RPC_E_CHANGED_MODE,
        Media::MediaFoundation::*,
        System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED},
    },
};

type Raw = *mut c_void;
type Create = unsafe extern "system" fn(*mut Raw) -> HRESULT;
type CreateBuffer = unsafe extern "system" fn(u32, *mut Raw) -> HRESULT;
type CreateSink = unsafe extern "system" fn(PCWSTR, Raw, Raw, *mut Raw) -> HRESULT;
type Shutdown = unsafe extern "system" fn() -> HRESULT;
type Startup = unsafe extern "system" fn(u32, u32) -> HRESULT;

struct Runtime {
    // The libraries outlive every interface, including failure during setup.
    _platform: Library,
    _read_write: Library,
    media_type: Create,
    sample: Create,
    buffer: CreateBuffer,
    sink: CreateSink,
    shutdown: Shutdown,
    initialized_com: bool,
}
impl Runtime {
    unsafe fn load() -> io::Result<Self> {
        let platform = Library::load_with_flags("mfplat.dll", LOAD_LIBRARY_SEARCH_SYSTEM32)
            .map_err(io::Error::other)?;
        let read_write = Library::load_with_flags("mfreadwrite.dll", LOAD_LIBRARY_SEARCH_SYSTEM32)
            .map_err(io::Error::other)?;
        let startup = *platform
            .get::<Startup>(b"MFStartup\0")
            .map_err(io::Error::other)?;
        let shutdown = *platform
            .get::<Shutdown>(b"MFShutdown\0")
            .map_err(io::Error::other)?;
        let media_type = *platform
            .get::<Create>(b"MFCreateMediaType\0")
            .map_err(io::Error::other)?;
        let sample = *platform
            .get::<Create>(b"MFCreateSample\0")
            .map_err(io::Error::other)?;
        let buffer = *platform
            .get::<CreateBuffer>(b"MFCreateMemoryBuffer\0")
            .map_err(io::Error::other)?;
        let sink = *read_write
            .get::<CreateSink>(b"MFCreateSinkWriterFromURL\0")
            .map_err(io::Error::other)?;
        let result = CoInitializeEx(None, COINIT_MULTITHREADED);
        if result != RPC_E_CHANGED_MODE {
            result.ok()?
        }
        if let Err(err) = startup(MF_VERSION, MFSTARTUP_FULL).ok() {
            if result.is_ok() {
                CoUninitialize()
            }
            return Err(err.into());
        }
        Ok(Self {
            _platform: platform,
            _read_write: read_write,
            media_type,
            sample,
            buffer,
            sink,
            shutdown,
            initialized_com: result.is_ok(),
        })
    }
    unsafe fn video_type(
        &self,
        subtype: &GUID,
        w: u32,
        h: u32,
        fps: u32,
    ) -> io::Result<IMFMediaType> {
        let mut raw = ptr::null_mut();
        (self.media_type)(&mut raw).ok()?;
        let ty = IMFMediaType::from_raw(raw);
        ty.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        ty.SetGUID(&MF_MT_SUBTYPE, subtype)?;
        ty.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        ty.SetUINT64(&MF_MT_FRAME_SIZE, (u64::from(w) << 32) | u64::from(h))?;
        ty.SetUINT64(&MF_MT_FRAME_RATE, (u64::from(fps) << 32) | 1)?;
        ty.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, (1 << 32) | 1)?;
        Ok(ty)
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        unsafe {
            let _ = (self.shutdown)();
            if self.initialized_com {
                CoUninitialize()
            }
        }
    }
}
pub(super) struct Encoder {
    writer: IMFSinkWriter,
    stream: u32,
    fps: u32,
    runtime: Runtime,
}
impl Encoder {
    pub(super) fn new(path: &Path, w: u32, h: u32, fps: u32) -> io::Result<Self> {
        let name: Vec<_> = path.as_os_str().encode_wide().chain([0]).collect();
        unsafe {
            let runtime = Runtime::load()?;
            let mut raw = ptr::null_mut();
            (runtime.sink)(
                PCWSTR(name.as_ptr()),
                ptr::null_mut(),
                ptr::null_mut(),
                &mut raw,
            )
            .ok()?;
            let writer = IMFSinkWriter::from_raw(raw);
            let output = runtime.video_type(&MFVideoFormat_H264, w, h, fps)?;
            let bitrate =
                (u64::from(w) * u64::from(h) * u64::from(fps) / 6).clamp(300_000, 80_000_000);
            output.SetUINT32(&MF_MT_AVG_BITRATE, bitrate as u32)?;
            let stream = writer.AddStream(&output)?;
            let input = runtime.video_type(&MFVideoFormat_RGB32, w, h, fps)?;
            // Positive stride denotes a top-down scan in Media Foundation.
            input.SetUINT32(&MF_MT_DEFAULT_STRIDE, w * 4)?;
            writer.SetInputMediaType(stream, &input, None)?;
            writer.BeginWriting()?;
            Ok(Self {
                writer,
                stream,
                fps,
                runtime,
            })
        }
    }
    pub(super) fn frame(&mut self, pixels: &[u8], index: u64) -> io::Result<()> {
        let size = u32::try_from(pixels.len()).map_err(io::Error::other)?;
        unsafe {
            let mut raw = ptr::null_mut();
            (self.runtime.buffer)(size, &mut raw).ok()?;
            let buffer = IMFMediaBuffer::from_raw(raw);
            let mut dest = ptr::null_mut();
            buffer.Lock(&mut dest, None, None)?;
            let bytes = std::slice::from_raw_parts_mut(dest, pixels.len());
            for (src, dst) in pixels.chunks_exact(4).zip(bytes.chunks_exact_mut(4)) {
                dst.copy_from_slice(&[src[2], src[1], src[0], 255]);
            }
            buffer.Unlock()?;
            buffer.SetCurrentLength(size)?;
            (self.runtime.sample)(&mut raw).ok()?;
            let sample = IMFSample::from_raw(raw);
            sample.AddBuffer(&buffer)?;
            let time = |i: u64| (i * 10_000_000 / u64::from(self.fps)) as i64;
            sample.SetSampleTime(time(index))?;
            sample.SetSampleDuration(time(index + 1) - time(index))?;
            self.writer.WriteSample(self.stream, &sample)?;
        }
        Ok(())
    }
    pub(super) fn finish(self, _: u64) -> io::Result<()> {
        Ok(unsafe { self.writer.Finalize()? })
    }
}
