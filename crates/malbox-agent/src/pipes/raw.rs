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

pub const BUFSIZE: u32 = 0x10000;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub struct NamedPipe(HANDLE);

unsafe impl Send for NamedPipe {}

impl NamedPipe {
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

    unsafe fn create(
        name: &str,
        open_mode: u32,
        pipe_mode: u32,
        out_buf: u32,
        in_buf: u32,
    ) -> anyhow::Result<Self> {
        let wname = wide(name);

        unsafe {
            let mut sd: SECURITY_DESCRIPTOR = std::mem::zeroed();
            InitializeSecurityDescriptor(&mut sd as *mut _ as PSECURITY_DESCRIPTOR, 1);
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
                    "creating pipe {} failed: error {}",
                    name,
                    GetLastError()
                ));
            }
            Ok(NamedPipe(handle))
        }
    }

    pub fn connect(&self) -> anyhow::Result<()> {
        let ok = unsafe { ConnectNamedPipe(self.0, ptr::null_mut()) };
        if ok == 0 {
            let err = unsafe { GetLastError() };
            if err != ERROR_PIPE_CONNECTED {
                return Err(anyhow::anyhow!("connecting to pipe failed: error {}", err));
            }
        }
        Ok(())
    }

    pub fn disconnect(&self) {
        unsafe {
            DisconnectNamedPipe(self.0);
        }
    }

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
                ERROR_BROKEN_PIPE => return Ok(0),
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
