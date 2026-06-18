//! Low-level Win32 named-pipe wrapper, mirroring CAPEv2's
//! `analyzer/windows/lib/core/pipe.py` (`PipeServer.run`).
//!
//! The point of dropping to raw `windows_sys` is the **null-DACL security
//! descriptor**: pipes created this way can be opened by capemon regardless of
//! the token/integrity level it ends up running under. `tokio`'s named-pipe
//! builder has no way to express that.
//!
//! [`NamedPipe`] owns a pipe `HANDLE`, closes it on drop, and implements
//! `std::io::Read`/`std::io::Write` over `ReadFile`/`WriteFile` so the existing
//! sync frame parsers in `bson_log.rs` can read straight from it on a blocking
//! thread.

use std::ffi::c_void;
use std::io::{self, Read, Write};
use std::ptr;

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_BROKEN_PIPE, ERROR_MORE_DATA, ERROR_PIPE_CONNECTED, FALSE, GetLastError,
    HANDLE, INVALID_HANDLE_VALUE, TRUE,
};
use windows_sys::Win32::Security::{
    InitializeSecurityDescriptor, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, SECURITY_DESCRIPTOR,
    SetSecurityDescriptorDacl,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, OPEN_EXISTING, PIPE_ACCESS_DUPLEX, PIPE_ACCESS_INBOUND, ReadFile, WriteFile,
};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE,
    PIPE_READMODE_MESSAGE, PIPE_TYPE_BYTE, PIPE_TYPE_MESSAGE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};

/// In/out buffer size, matching CAPE's `BUFSIZE = 0x10000`.
pub const BUFSIZE: u32 = 0x10000;

/// Encode a Rust string as a NUL-terminated UTF-16 buffer for the `*W` Win32 API.
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Owned named-pipe server handle.
pub struct NamedPipe(HANDLE);

// A pipe HANDLE is just a kernel object handle; it is safe to hand a `NamedPipe`
// to a dedicated blocking thread (which is exactly how `command`/`logserver`
// use it). The raw pointer inside makes it `!Send` by default.
unsafe impl Send for NamedPipe {}

impl NamedPipe {
    /// Command pipe: duplex, message framing — capemon writes a command and
    /// reads our response. Matches CAPE's `message=True` branch.
    pub fn create_command(name: &str) -> anyhow::Result<Self> {
        unsafe {
            Self::create(
                name,
                PIPE_ACCESS_DUPLEX,
                PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT,
                BUFSIZE,
                BUFSIZE,
            )
        }
    }

    /// Log pipe: inbound-only byte stream the monitor writes BSON frames into.
    /// Matches CAPE's `message=False` branch.
    pub fn create_log(name: &str) -> anyhow::Result<Self> {
        unsafe {
            Self::create(
                name,
                PIPE_ACCESS_INBOUND,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                0,
                BUFSIZE,
            )
        }
    }

    /// Build a null-DACL `SECURITY_ATTRIBUTES` and create one pipe instance.
    ///
    /// `sd`/`sa` only need to outlive the `CreateNamedPipeW` call — it copies
    /// the security info — so keeping them on this stack frame is correct.
    unsafe fn create(
        name: &str,
        open_mode: u32,
        pipe_mode: u32,
        out_buf: u32,
        in_buf: u32,
    ) -> anyhow::Result<Self> {
        let wname = wide(name);

        // SAFETY: caller invokes `create` in an `unsafe` context; all the FFI
        // below operates on stack locals that outlive the CreateNamedPipeW call,
        // which copies the security info it needs.
        unsafe {
            let mut sd: SECURITY_DESCRIPTOR = std::mem::zeroed();
            // SECURITY_DESCRIPTOR_REVISION == 1
            InitializeSecurityDescriptor(&mut sd as *mut _ as PSECURITY_DESCRIPTOR, 1);
            // present=TRUE, dacl=NULL  ->  null DACL == everyone may open the pipe
            SetSecurityDescriptorDacl(
                &mut sd as *mut _ as PSECURITY_DESCRIPTOR,
                TRUE,
                ptr::null(),
                FALSE,
            );
            let mut sa = SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: &mut sd as *mut _ as *mut c_void,
                bInheritHandle: FALSE,
            };

            let handle = CreateNamedPipeW(
                wname.as_ptr(),
                open_mode,
                pipe_mode,
                PIPE_UNLIMITED_INSTANCES,
                out_buf,
                in_buf,
                0,
                &mut sa,
            );

            if handle == INVALID_HANDLE_VALUE {
                return Err(anyhow::anyhow!(
                    "CreateNamedPipeW({}) failed: error {}",
                    name,
                    GetLastError()
                ));
            }
            Ok(NamedPipe(handle))
        }
    }

    /// Block until a client connects. An already-connected client
    /// (`ERROR_PIPE_CONNECTED`) is success, matching CAPE.
    pub fn connect(&self) -> anyhow::Result<()> {
        let ok = unsafe { ConnectNamedPipe(self.0, ptr::null_mut()) };
        if ok == 0 {
            let err = unsafe { GetLastError() };
            if err != ERROR_PIPE_CONNECTED {
                return Err(anyhow::anyhow!("ConnectNamedPipe failed: error {}", err));
            }
        }
        Ok(())
    }

    /// Detach the current client so the same instance can accept another.
    pub fn disconnect(&self) {
        unsafe {
            DisconnectNamedPipe(self.0);
        }
    }

    /// Raw handle, for the few callers that need it directly.
    pub fn handle(&self) -> HANDLE {
        self.0
    }
}

impl Read for NamedPipe {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut n: u32 = 0;
        let ok = unsafe {
            ReadFile(
                self.0,
                buf.as_mut_ptr(),
                buf.len() as u32,
                &mut n,
                ptr::null_mut(),
            )
        };
        if ok == 0 {
            let err = unsafe { GetLastError() };
            match err {
                // Clean end-of-stream: read_exact -> UnexpectedEof, which
                // read_bson_frame already maps to Ok(None).
                ERROR_BROKEN_PIPE => return Ok(0),
                // Message-mode partial read: the bytes we got are valid; the
                // caller can read the rest on the next call.
                ERROR_MORE_DATA => return Ok(n as usize),
                _ => return Err(io::Error::from_raw_os_error(err as i32)),
            }
        }
        Ok(n as usize)
    }
}

impl Write for NamedPipe {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut n: u32 = 0;
        let ok = unsafe {
            WriteFile(
                self.0,
                buf.as_ptr(),
                buf.len() as u32,
                &mut n,
                ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(io::Error::from_raw_os_error(
                unsafe { GetLastError() } as i32
            ));
        }
        Ok(n as usize)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for NamedPipe {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

/// Open a one-shot client handle to `name` and immediately close it. Used on
/// shutdown to release a server thread blocked in `ConnectNamedPipe`.
pub fn nudge_server(name: &str) {
    const GENERIC_READ_WRITE: u32 = 0x8000_0000 | 0x4000_0000;
    let wname = wide(name);
    let h = unsafe {
        CreateFileW(
            wname.as_ptr(),
            GENERIC_READ_WRITE,
            0,
            ptr::null(),
            OPEN_EXISTING,
            0,
            ptr::null_mut(),
        )
    };
    if h != INVALID_HANDLE_VALUE {
        unsafe {
            CloseHandle(h);
        }
    }
}
