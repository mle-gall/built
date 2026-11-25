//! Forward Explorer opens to the shell in the same Windows logon session.
//! Reserve the first pipe instance before Python starts so simultaneous
//! launches elect one owner. Custom profiles never enter this path.
use super::{absolute, EventLoopProxy, UserEvent};
use std::{
    ffi::{OsStr, OsString},
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        fs::OpenOptionsExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::{Path, PathBuf},
};
use windows::{
    core::{HSTRING, PWSTR},
    Win32::{
        Foundation::{
            LocalFree, ERROR_ACCESS_DENIED, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, HANDLE, HLOCAL,
        },
        Security::{
            Authorization::{
                ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
                SDDL_REVISION_1,
            },
            GetTokenInformation, TokenLogonSid, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES,
            TOKEN_GROUPS, TOKEN_QUERY,
        },
        Storage::FileSystem::{
            FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX, SECURITY_IDENTIFICATION,
            SECURITY_SQOS_PRESENT,
        },
        System::{
            Pipes::*,
            Threading::{
                GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
            },
        },
        UI::WindowsAndMessaging::AllowSetForegroundWindow,
    },
};

const MAX_UNITS: usize = 32767;

fn logon_sid(process: HANDLE) -> io::Result<String> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(process, TOKEN_QUERY, &mut token)?;
        let owned_token = OwnedHandle::from_raw_handle(token.0);
        let token = HANDLE(owned_token.as_raw_handle());
        let mut size = 0;
        let _ = GetTokenInformation(token, TokenLogonSid, None, 0, &mut size);
        if size == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut buf = vec![0usize; (size as usize).div_ceil(size_of::<usize>())];
        GetTokenInformation(
            token,
            TokenLogonSid,
            Some(buf.as_mut_ptr().cast()),
            size,
            &mut size,
        )?;
        let groups = &*buf.as_ptr().cast::<TOKEN_GROUPS>();
        if groups.GroupCount != 1 {
            return Err(io::Error::other("missing logon SID"));
        }
        let mut sid = PWSTR::null();
        ConvertSidToStringSidW(groups.Groups[0].Sid, &mut sid)?;
        let text = sid.to_string();
        LocalFree(HLOCAL(sid.0.cast()));
        text.map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }
}

pub enum Instance {
    Primary(File),
    Forwarded,
}

pub fn acquire(file: Option<&Path>) -> io::Result<Instance> {
    let sid = logon_sid(unsafe { GetCurrentProcess() })?;
    acquire_named(&format!(r"\\.\pipe\Built-{sid}"), &sid, file)
}

fn acquire_named(name: &str, sid: &str, file: Option<&Path>) -> io::Result<Instance> {
    let wide = HSTRING::from(name);
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    let handle = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            &HSTRING::from(format!("D:P(A;;GA;;;{sid})")),
            SDDL_REVISION_1,
            &mut descriptor,
            None,
        )?;
        let security = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor.0,
            ..Default::default()
        };
        let handle = CreateNamedPipeW(
            &wide,
            PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_BYTE | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            1,
            (MAX_UNITS * 2 + 4) as u32,
            1000,
            Some(&security),
        );
        let error = io::Error::last_os_error();
        LocalFree(HLOCAL(descriptor.0));
        if handle.is_invalid() {
            Err(error)
        } else {
            Ok(File::from_raw_handle(handle.0))
        }
    };
    match handle {
        Ok(server) => Ok(Instance::Primary(server)),
        Err(e)
            if e.raw_os_error() == Some(ERROR_ACCESS_DENIED.0 as i32)
                || e.raw_os_error() == Some(ERROR_PIPE_BUSY.0 as i32) =>
        {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let mut client = loop {
                match OpenOptions::new()
                    .read(true)
                    .write(true)
                    .security_qos_flags((SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION).0)
                    .open(name)
                {
                    Ok(pipe) => break pipe,
                    Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY.0 as i32) => {
                        let remaining =
                            deadline.saturating_duration_since(std::time::Instant::now());
                        if remaining.is_zero()
                            || !unsafe {
                                WaitNamedPipeW(&wide, remaining.as_millis().max(1) as u32)
                            }
                            .as_bool()
                        {
                            return Err(e);
                        }
                    }
                    Err(e) => return Err(e),
                }
            };
            unsafe {
                let mut pid = 0;
                GetNamedPipeServerProcessId(HANDLE(client.as_raw_handle()), &mut pid)?;
                let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)?;
                let process = OwnedHandle::from_raw_handle(process.0);
                if logon_sid(HANDLE(process.as_raw_handle()))? != sid {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "different logon session",
                    ));
                }
                let _ = AllowSetForegroundWindow(pid);
            }
            let path = file.map(absolute);
            let units: Vec<u16> = path
                .as_deref()
                .map(Path::as_os_str)
                .unwrap_or(OsStr::new(""))
                .encode_wide()
                .collect();
            if units.len() > MAX_UNITS {
                return Err(io::Error::other("path too long"));
            }
            client.write_all(&(units.len() as u32).to_le_bytes())?;
            client.write_all(
                &units
                    .into_iter()
                    .flat_map(u16::to_le_bytes)
                    .collect::<Vec<_>>(),
            )?;
            let mut ack = [0];
            client.read_exact(&mut ack)?;
            if ack != [1] {
                return Err(io::Error::other("shell rejected open"));
            }
            Ok(Instance::Forwarded)
        }
        Err(e) => Err(e),
    }
}

fn receive(pipe: &mut File) -> io::Result<UserEvent> {
    let mut count = [0; 4];
    pipe.read_exact(&mut count)?;
    let units = u32::from_le_bytes(count) as usize;
    if units == 0 {
        return Ok(UserEvent::Menu("activate".into()));
    }
    if units > MAX_UNITS {
        return Err(io::Error::other("invalid path length"));
    }
    let mut bytes = vec![0; units * 2];
    pipe.read_exact(&mut bytes)?;
    let wide: Vec<_> = bytes
        .chunks_exact(2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .collect();
    Ok(UserEvent::Open(PathBuf::from(OsString::from_wide(&wide))))
}

// Returns false when the event loop has already exited.
fn serve_one(pipe: &mut File, dispatch: impl FnOnce(UserEvent) -> bool) -> io::Result<bool> {
    let handle = HANDLE(pipe.as_raw_handle());
    if let Err(e) = unsafe { ConnectNamedPipe(handle, None) } {
        if e.code() != ERROR_PIPE_CONNECTED.to_hresult() {
            return Err(e.into());
        }
    }
    let accepted = receive(pipe).map(dispatch)?;
    if accepted {
        pipe.write_all(&[1])?;
        // Disconnect discards outstanding bytes. Deliver the acknowledgement
        // before the listener reuses its single pipe instance.
        unsafe {
            windows::Win32::Storage::FileSystem::FlushFileBuffers(handle)?;
        }
    }
    Ok(accepted)
}

pub fn listen(mut pipe: File, proxy: EventLoopProxy<UserEvent>) {
    std::thread::spawn(move || loop {
        let result = serve_one(&mut pipe, |event| proxy.send_event(event).is_ok());
        let _ = unsafe { DisconnectNamedPipe(HANDLE(pipe.as_raw_handle())) };
        if matches!(result, Ok(false)) {
            break;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_clients_forward_activation_and_a_unicode_document() {
        let sid = logon_sid(unsafe { GetCurrentProcess() }).unwrap();
        let name = format!(r"\\.\pipe\Built-test-{}-{sid}", std::process::id());
        let Instance::Primary(mut pipe) = acquire_named(&name, &sid, None).unwrap() else {
            panic!("test name in use")
        };
        let path = Path::new("Projet d’été 東京.built");
        let server = std::thread::spawn(move || {
            let mut events = Vec::new();
            for _ in 0..2 {
                assert!(serve_one(&mut pipe, |event| {
                    events.push(event);
                    true
                })
                .unwrap());
                unsafe {
                    DisconnectNamedPipe(HANDLE(pipe.as_raw_handle())).unwrap();
                }
            }
            events
        });
        assert!(matches!(
            acquire_named(&name, &sid, None).unwrap(),
            Instance::Forwarded
        ));
        assert!(matches!(
            acquire_named(&name, &sid, Some(path)).unwrap(),
            Instance::Forwarded
        ));
        let events = server.join().unwrap();
        assert!(matches!(&events[0], UserEvent::Menu(cmd) if cmd=="activate"));
        assert!(matches!(&events[1], UserEvent::Open(p) if p==&absolute(path)));
    }
}
