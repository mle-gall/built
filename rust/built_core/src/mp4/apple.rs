//! AVAssetWriter encodes H.264 through the system media stack. The pixel
//! buffer adaptor retains each submitted buffer until compression is done.
use objc2::{
    class, msg_send,
    rc::{autoreleasepool, Retained},
    runtime::AnyObject,
    Encode, Encoding, RefEncode,
};
use objc2_foundation::{NSDictionary, NSError, NSNumber, NSString, NSURL};
use std::{
    ffi::c_void,
    io,
    path::Path,
    ptr::{self, NonNull},
    time::{Duration, Instant},
};

#[repr(C)]
#[derive(Clone, Copy)]
struct Time {
    value: i64,
    scale: i32,
    flags: u32,
    epoch: i64,
}
// CMTime is an anonymous C struct in CoreMedia/CMTime.h.
unsafe impl Encode for Time {
    const ENCODING: Encoding = Encoding::Struct(
        "?",
        &[i64::ENCODING, i32::ENCODING, u32::ENCODING, i64::ENCODING],
    );
}
#[repr(C)]
struct Buffer {
    _opaque: [u8; 0],
}
unsafe impl RefEncode for Buffer {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("__CVBuffer", &[]));
}
#[link(name = "AVFoundation", kind = "framework")]
extern "C" {
    static AVFileTypeMPEG4: &'static NSString;
    static AVMediaTypeVideo: &'static NSString;
    static AVVideoCodecKey: &'static NSString;
    static AVVideoCodecTypeH264: &'static NSString;
    static AVVideoWidthKey: &'static NSString;
    static AVVideoHeightKey: &'static NSString;
}
#[link(name = "CoreVideo", kind = "framework")]
extern "C" {
    fn CVPixelBufferCreate(
        allocator: *const c_void,
        w: usize,
        h: usize,
        format: u32,
        attributes: *const c_void,
        buffer: *mut *mut Buffer,
    ) -> i32;
    fn CVPixelBufferLockBaseAddress(buffer: *mut Buffer, flags: usize) -> i32;
    fn CVPixelBufferGetBaseAddress(buffer: *mut Buffer) -> *mut u8;
    fn CVPixelBufferGetBytesPerRow(buffer: *mut Buffer) -> usize;
    fn CVPixelBufferUnlockBaseAddress(buffer: *mut Buffer, flags: usize) -> i32;
    fn CVPixelBufferRelease(buffer: *mut Buffer);
}

struct PixelBuffer(NonNull<Buffer>);
impl Drop for PixelBuffer {
    fn drop(&mut self) {
        unsafe { CVPixelBufferRelease(self.0.as_ptr()) }
    }
}
impl PixelBuffer {
    fn rgba(width: usize, height: usize, pixels: &[u8]) -> io::Result<Self> {
        unsafe {
            let mut raw = ptr::null_mut();
            let status = CVPixelBufferCreate(
                ptr::null(),
                width,
                height,
                u32::from_be_bytes(*b"BGRA"),
                ptr::null(),
                &mut raw,
            );
            let buffer = NonNull::new(raw)
                .filter(|_| status == 0)
                .map(Self)
                .ok_or_else(|| {
                    io::Error::other(format!("Creating a video pixel buffer failed ({status})"))
                })?;
            if CVPixelBufferLockBaseAddress(raw, 0) != 0 {
                return Err(io::Error::other("Locking a video buffer failed"));
            }
            let base = CVPixelBufferGetBaseAddress(raw);
            let stride = CVPixelBufferGetBytesPerRow(raw);
            if base.is_null() || stride < width * 4 {
                CVPixelBufferUnlockBaseAddress(raw, 0);
                return Err(io::Error::other("Invalid video buffer layout"));
            }
            for (y, row) in pixels.chunks_exact(width * 4).enumerate() {
                let dest = std::slice::from_raw_parts_mut(base.add(y * stride), width * 4);
                for (src, dst) in row.chunks_exact(4).zip(dest.chunks_exact_mut(4)) {
                    dst.copy_from_slice(&[src[2], src[1], src[0], src[3]]);
                }
            }
            CVPixelBufferUnlockBaseAddress(raw, 0);
            Ok(buffer)
        }
    }
}
pub(super) struct Encoder {
    writer: Retained<AnyObject>,
    input: Retained<AnyObject>,
    adaptor: Retained<AnyObject>,
    width: usize,
    height: usize,
    fps: i32,
}
impl Encoder {
    fn time(&self, frame: u64) -> Time {
        Time {
            value: frame as i64,
            scale: self.fps,
            flags: 1,
            epoch: 0,
        }
    }
    fn error(&self) -> io::Error {
        let error: Option<Retained<NSError>> = unsafe { msg_send![&*self.writer, error] };
        io::Error::other(error.map_or_else(
            || "AVAssetWriter could not encode H.264".into(),
            |e| e.localizedDescription().to_string(),
        ))
    }
    pub(super) fn new(path: &Path, width: u32, height: u32, fps: u32) -> io::Result<Self> {
        autoreleasepool(|_| unsafe {
            let url = NSURL::from_file_path(path)
                .ok_or_else(|| io::Error::other("Invalid video destination"))?;
            let mut error = None::<Retained<NSError>>;
            let writer: Option<Retained<AnyObject>> = msg_send![class!(AVAssetWriter), assetWriterWithURL:&*url, fileType:AVFileTypeMPEG4, error:&mut error];
            let writer = writer.ok_or_else(|| {
                io::Error::other(error.map_or_else(
                    || "Opening MP4 output failed".into(),
                    |e| e.localizedDescription().to_string(),
                ))
            })?;
            let settings = NSDictionary::<NSString, AnyObject>::from_slices(
                &[AVVideoCodecKey, AVVideoWidthKey, AVVideoHeightKey],
                &[
                    AVVideoCodecTypeH264.as_ref(),
                    NSNumber::new_u32(width).as_ref(),
                    NSNumber::new_u32(height).as_ref(),
                ],
            );
            let applies: bool =
                msg_send![&*writer,canApplyOutputSettings:&*settings,forMediaType:AVMediaTypeVideo];
            if !applies {
                return Err(io::Error::other("H.264 settings unavailable"));
            }
            let input: Retained<AnyObject> = msg_send![class!(AVAssetWriterInput),assetWriterInputWithMediaType:AVMediaTypeVideo,outputSettings:&*settings];
            let adaptor: Retained<AnyObject> = msg_send![class!(AVAssetWriterInputPixelBufferAdaptor),assetWriterInputPixelBufferAdaptorWithAssetWriterInput:&*input,sourcePixelBufferAttributes:None::<&AnyObject>];
            let can_add: bool = msg_send![&*writer,canAddInput:&*input];
            if !can_add {
                return Err(io::Error::other("Cannot add H.264 track"));
            }
            let _: () = msg_send![&*writer,addInput:&*input];
            let result = Self {
                writer,
                input,
                adaptor,
                width: width as usize,
                height: height as usize,
                fps: fps as i32,
            };
            let started: bool = msg_send![&*result.writer, startWriting];
            if !started {
                return Err(result.error());
            }
            let _: () = msg_send![&*result.writer,startSessionAtSourceTime:result.time(0)];
            Ok(result)
        })
    }
    pub(super) fn frame(&mut self, pixels: &[u8], index: u64) -> io::Result<()> {
        autoreleasepool(|_| unsafe {
            let start = Instant::now();
            loop {
                let ready: bool = msg_send![&*self.input, isReadyForMoreMediaData];
                if ready {
                    break;
                }
                let status: isize = msg_send![&*self.writer, status];
                if status != 1 || start.elapsed() > Duration::from_secs(30) {
                    return Err(self.error());
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            let buffer = PixelBuffer::rgba(self.width, self.height, pixels)?;
            let ok: bool = msg_send![&*self.adaptor,appendPixelBuffer:buffer.0.as_ptr(),withPresentationTime:self.time(index)];
            if !ok {
                return Err(self.error());
            }
            Ok(())
        })
    }
    pub(super) fn finish(self, frames: u64) -> io::Result<()> {
        autoreleasepool(|_| unsafe {
            let _: () = msg_send![&*self.writer,endSessionAtSourceTime:self.time(frames)];
            let _: () = msg_send![&*self.input, markAsFinished];
            let (done, completed) = std::sync::mpsc::channel();
            let completion = block2::RcBlock::new(move || {
                let _ = done.send(());
            });
            let _: () = msg_send![&*self.writer, finishWritingWithCompletionHandler: &*completion];
            completed.recv().map_err(io::Error::other)?;
            let status: isize = msg_send![&*self.writer, status];
            if status != 2 {
                // AVAssetWriterStatusCompleted
                return Err(self.error());
            }
            Ok(())
        })
    }
}
impl Drop for Encoder {
    fn drop(&mut self) {
        autoreleasepool(|_| unsafe {
            let status: isize = msg_send![&*self.writer, status];
            if status == 1 {
                let _: () = msg_send![&*self.writer, cancelWriting];
            }
        });
    }
}
