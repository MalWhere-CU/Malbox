use core::slice;
use std::fs::File;
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};
use std::{
    io::{Read, Write},
    path::Path,
    ptr,
};

use std::ffi::{CString, OsStr};

use std::os::windows::ffi::OsStrExt;

use rand::RngExt;
use windows_sys::Wdk::System::Threading::NtQueryInformationProcess;
use windows_sys::Win32::Foundation::{CloseHandle, FALSE, NTSTATUS, STATUS_INVALID_INFO_CLASS};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetExitCodeProcess, OpenProcess, OpenThread, PROCESS_ALL_ACCESS,
    PROCESS_QUERY_LIMITED_INFORMATION, ResumeThread, Sleep, THREAD_ALL_ACCESS,
};
use windows_sys::{
    Win32::{
        Foundation::{GetLastError, HANDLE},
        System::Threading::{
            CREATE_NEW_CONSOLE, CREATE_SUSPENDED, CreateProcessW, IsWow64Process,
            PROCESS_INFORMATION, STARTF_USESHOWWINDOW, STARTUPINFOW,
        },
        UI::WindowsAndMessaging::SW_SHOWNORMAL,
    },
    core::{BOOL, PCWSTR, PWSTR},
};
#[repr(C)]
struct UNICODE_STRING {
    length: u16,
    maximum_length: u16,
    buffer: *mut u16,
}
use crate::monitor::MonitorState;

static PROCESS_NUM: AtomicU32 = AtomicU32::new(0);
pub struct Process {
    pub pid: u32,
    pub tid: u32,
    pub h_process: Option<HANDLE>,
    pub h_thread: Option<HANDLE>,
    pub suspended: bool,
}

impl Process {
    pub fn open(&mut self) -> anyhow::Result<bool> {
        let mut ret = self.pid != 0 || self.tid != 0;

        if self.pid != 0 && self.h_process.is_none() {
            let current_pid = std::process::id();
            if self.pid == current_pid {
                unsafe {
                    self.h_process = Some(GetCurrentProcess());
                }
            } else {
                unsafe {
                    self.h_process = Some(OpenProcess(PROCESS_ALL_ACCESS, FALSE, self.pid));
                    if self.h_process.unwrap().is_null() {
                        self.h_process = Some(OpenProcess(
                            PROCESS_QUERY_LIMITED_INFORMATION,
                            FALSE,
                            self.pid,
                        ));
                    }
                }
            }
            ret = true;
            if let Some(h) = self.h_process {
                if h.is_null() {
                    return Err(anyhow::anyhow!("failed to open process {}", self.pid));
                }
            }
        }

        if self.tid != 0 && self.h_thread.is_none() {
            unsafe {
                self.h_thread = Some(OpenThread(THREAD_ALL_ACCESS, FALSE, self.tid));
            }
            if let Some(h) = self.h_thread {
                if h.is_null() {
                    return Err(anyhow::anyhow!(
                        "OpenThread(THREAD_ALL_ACCESS, ...) failed for thread {}",
                        self.tid
                    ));
                }
            }
            ret = true;
        }
        Ok(ret)
    }
    pub fn is_64bit(&self) -> anyhow::Result<bool> {
        if let Some(h_process) = self.h_process {
            if !h_process.is_null() {
                unsafe {
                    let mut is_wow64: BOOL = 0;
                    let ret = IsWow64Process(h_process, &mut is_wow64) != 0;
                    return Ok(ret && is_wow64 == 0);
                }
            }
        }
        Err(anyhow::anyhow!(
            "the process of id {} does not have an open handle.",
            self.pid
        ))
    }
    pub fn execute(
        &mut self,
        path: &Path,
        args: Option<&str>,
        suspended: bool,
    ) -> anyhow::Result<()> {
        if !path.exists() {
            return Err(anyhow::anyhow!(
                "failed to access file at path {}",
                path.to_string_lossy()
            ));
        }
        let mut startup_info = STARTUPINFOW::default();
        startup_info.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
        startup_info.dwFlags = STARTF_USESHOWWINDOW;
        startup_info.wShowWindow = SW_SHOWNORMAL as u16;
        let mut process_info = PROCESS_INFORMATION::default();
        let mut arguments = format!("\"{}\" ", path.to_string_lossy());
        if let Some(a) = args {
            arguments.push_str(a);
        }
        let mut creation_flags = CREATE_NEW_CONSOLE;
        if suspended {
            self.suspended = true;
            creation_flags |= CREATE_SUSPENDED;
        }
        let execution_directory = "C:\\Users\\mohamed\\Desktop\\sample";
        let path_w: Vec<u16> = OsStr::new(path)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let args_w: Vec<u16> = OsStr::new(&arguments)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let dir_w: Vec<u16> = OsStr::new(&execution_directory)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        unsafe {
            let created: bool = CreateProcessW(
                PCWSTR::from(path_w.as_ptr()),
                PWSTR::from(args_w.as_ptr() as *mut u16),
                ptr::null(),
                ptr::null(),
                0,
                creation_flags,
                ptr::null(),
                PCWSTR::from(dir_w.as_ptr()),
                &startup_info,
                &mut process_info,
            ) != 0;
            if created {
                self.pid = process_info.dwProcessId;
                self.h_process = Some(process_info.hProcess);
                self.tid = process_info.dwThreadId;
                self.h_thread = Some(process_info.hThread);
                println!(
                    "Successfully executed process from path \"{}\" with pid {}",
                    path.display(),
                    self.pid
                );
                return Ok(());
            } else {
                let err = GetLastError();
                return Err(anyhow::anyhow!(
                    "Failed to execute process from path \"{}\" (Error Code: {})",
                    path.display(),
                    err
                ));
            }
        }
    }
    pub fn exit_code(&mut self) -> Option<u32> {
        if self.h_process.is_none() || self.h_process.unwrap().is_null() {
            self.open();
        }

        let mut exit_code: u32 = 0;
        unsafe {
            let ok = GetExitCodeProcess(self.h_process.unwrap(), &mut exit_code) != 0;
            if !ok {
                println!("Failed getting exit code for process {}", self.pid);
                return None;
            }
        }
        Some(exit_code)
    }
    pub fn is_alive(&mut self) -> bool {
        if let Some(code) = self.exit_code() {
            code == 0x00000103
        } else {
            false
        }
    }
    pub fn get_filepath(&mut self) -> String {
        if self.h_process.is_none() || self.h_process.unwrap().is_null() {
            self.open();
        }

        let mut pbi = vec![0u8; 1024];
        let mut size: u32 = 0;

        unsafe {
            let ret = NtQueryInformationProcess(
                self.h_process.unwrap(),
                43,
                pbi.as_mut_ptr() as *mut _,
                pbi.len() as u32,
                &mut size,
            );

            if ret == STATUS_INVALID_INFO_CLASS {
                println!("windows does not support this information class");
                return String::new();
            }
            if ret >= 0 {
                // Cast the beginning of the byte buffer to our UNICODE_STRING struct
                let uni_str = &*(pbi.as_ptr() as *const UNICODE_STRING);

                // Ensure the pointer is valid and the length is greater than 0
                if !uni_str.buffer.is_null() && uni_str.length > 0 {
                    // Length is in BYTES. UTF-16 characters are 2 bytes each.
                    let char_count = (uni_str.length / 2) as usize;

                    // Create a Rust slice from the raw UTF-16 pointer
                    let utf16_slice = slice::from_raw_parts(uni_str.buffer, char_count);

                    // Convert the UTF-16 slice into a standard Rust String
                    return String::from_utf16_lossy(utf16_slice);
                }
            }
        }
        String::new()
    }
    fn write_monitor_config(
        &mut self,
        interest: Option<&str>,
        nosleepskip: bool,
        monitor_state: &MonitorState,
    ) -> anyhow::Result<()> {
        let config_path = monitor_state
            .binaries
            .work_dir
            .join(format!("dll\\{}.ini", self.pid));
        println!(
            "Monitor config for process {}: {}",
            self.pid,
            config_path.display()
        );
        let logserver_path = format!("{}{}", monitor_state.log_pipe_prefix, self.pid);
        let firstproc = PROCESS_NUM.fetch_add(1, Ordering::SeqCst) == 0;
        let interest_str = interest.unwrap_or("");
        let mut config = File::create(&config_path).expect("Failed to create config file");
        writeln!(config, "host-ip=127.0.0.1");
        writeln!(config, "host-port={}", 5000); // capemon reads them but never uses them.
        writeln!(config, "pipe={}", monitor_state.command_pipe);
        writeln!(config, "logserver={}", logserver_path);
        writeln!(
            config,
            "results=C:\\Users\\mohamed\\Desktop\\sample\\results"
        );

        //TODO:
        // this will create a problem where the malware sample will not have access to files in its
        // own directory
        writeln!(config, "analyzer=C:\\Users\\mohamed\\Desktop\\sample"); // we should put the
        // sample somewhere else
        // and regulate the
        // creation of analyzers
        // directory randomly.

        writeln!(config, "pythonpath=");
        writeln!(config, "first-process={}", if firstproc { 1 } else { 0 });
        writeln!(
            config,
            "startup-time={}",
            rand::rng().random_range(1..=30) * 20 * 60 * 1000
        );
        writeln!(config, "file-of-interest={}", interest_str);
        writeln!(config, "shutdown-mutex={}", monitor_state.shutdown_mutex);
        writeln!(
            config,
            "terminate-event={}{}",
            monitor_state.terminate_event_prefix, self.pid
        );
        if !nosleepskip
            && interest_str.len() > 2
            && !interest_str.starts_with("\\:")
            && PROCESS_NUM.load(Ordering::SeqCst) <= 2
        {
            writeln!(config, "force-sleepskip=0");
        }
        Ok(())
    }
    pub fn inject(
        &mut self,
        interest: Option<&str>,
        nosleepskip: bool,
        monitor_state: &MonitorState,
    ) -> anyhow::Result<()> {
        if self.pid == 0 {
            return Err(anyhow::anyhow!("invalid PID"));
        }
        if !self.is_alive() {
            return Err(anyhow::anyhow!(
                "the process {} is not alive, injection aborted",
                self.pid
            ));
        }
        if self.h_process == None {
            match self.open() {
                Ok(b) => {
                    if !b {
                        return Err(anyhow::anyhow!(
                            "failed to open process {} handle",
                            self.pid
                        ));
                    }
                }
                Err(e) => {
                    return Err(anyhow::anyhow!(
                        "failed to open process {} handle: {}",
                        self.pid,
                        e
                    ));
                }
            }
        }

        let (loader_name, dll, bit_str) = if self.is_64bit().unwrap() {
            (
                monitor_state.binaries.loader64.clone(),
                monitor_state.binaries.capemon64.clone(),
                "64-bit",
            )
        } else {
            (
                monitor_state.binaries.loader32.clone(),
                monitor_state.binaries.capemon32.clone(),
                "32-bit",
            )
        };

        if !loader_name.exists() {
            // this should never happen
            return Err(anyhow::anyhow!(
                "invalid loader path {} for injecting DLL, injection aborted",
                loader_name.display()
            ));
        }

        if !dll.exists() {
            return Err(anyhow::anyhow!(
                "invalid path {} for monitor DLL to be injected, injection aborted",
                dll.display()
            ));
        }

        if let Err(e) = self.write_monitor_config(interest, nosleepskip, monitor_state) {
            return Err(anyhow::anyhow!("Failed to write monitor config:\n  {}", e));
        }

        println!(
            "{} DLL to inject is {}, loader {}",
            bit_str,
            dll.display(),
            loader_name.display()
        );

        println!(
            "{} inject {} {} {}",
            loader_name.display(),
            self.pid.to_string(),
            self.tid.to_string(),
            dll.display()
        );
        match Command::new(loader_name)
            .arg("inject")
            .arg(self.pid.to_string())
            .arg(self.tid.to_string())
            .arg(dll)
            .output()
        {
            Ok(output) => {
                if output.status.code().unwrap() == 1 {
                    println!("Injected into {} process {}", bit_str, self.pid);
                } else {
                    return Err(anyhow::anyhow!(
                        "Unable to inject into {} process {}, error: {}",
                        bit_str,
                        self.pid,
                        output.status
                    ));
                }
            }
            Err(e) => {
                return Err(anyhow::anyhow!("Error running process: {}", e));
            }
        }

        Ok(())
    }
    pub fn resume(&mut self) -> anyhow::Result<bool> {
        if !self.suspended {
            return Ok(false);
        }
        if self.h_thread.is_none() {
            return Err(anyhow::anyhow!("thread handle is not open"));
        }
        unsafe {
            Sleep(2000);
            if ResumeThread(self.h_thread.unwrap()) != (-1i32 as u32) {
                self.suspended = false;
                println!("successfully resumed process {}", self.pid);
                Ok(true)
            } else {
                Err(anyhow::anyhow!("failed to resume process {}", self.pid))
            }
        }
    }
    pub fn close(&mut self) -> anyhow::Result<()> {
        unsafe {
            if let Some(h_process) = self.h_process.take() {
                if CloseHandle(h_process) == 0 {
                    return Err(anyhow::anyhow!(
                        "could not close process handle: {}",
                        self.pid
                    ));
                }
            } else {
                println!("process handle {} was already closed", self.pid);
            }
            if let Some(h_thread) = self.h_thread.take() {
                if CloseHandle(h_thread) == 0 {
                    return Err(anyhow::anyhow!(
                        "could not close thread handle: {}",
                        self.tid
                    ));
                }
            } else {
                println!("thread handle {} was already closed", self.tid);
            }
        }

        Ok(())
    }
    pub fn terminate_and_close(&mut self) {
        if let Some(h) = self.h_process {
            if !h.is_null() {
                unsafe {
                    windows_sys::Win32::System::Threading::TerminateProcess(h, 1);
                    windows_sys::Win32::Foundation::CloseHandle(h);
                }
            }
        }
        if let Some(h) = self.h_thread {
            if !h.is_null() {
                unsafe {
                    windows_sys::Win32::Foundation::CloseHandle(h);
                }
            }
        }
        self.h_process = None;
        self.h_thread = None;
        self.suspended = false;
    }
    pub fn set_terminate_event(&self, terminate_event_prefix: &str) -> anyhow::Result<()> {
        let event_name = format!("{}{}", terminate_event_prefix, self.pid);
        let c_event_name = CString::new(event_name.as_str())?;
        unsafe {
            let h = windows_sys::Win32::System::Threading::OpenEventW(
                windows_sys::Win32::System::Threading::EVENT_MODIFY_STATE,
                windows_sys::Win32::Foundation::FALSE,
                c_event_name.as_ptr() as *const u16,
            );
            if !h.is_null() {
                windows_sys::Win32::System::Threading::SetEvent(h);
                windows_sys::Win32::Foundation::CloseHandle(h);
                println!("[process] Terminate event set for pid {}", self.pid);
            } else {
                eprintln!(
                    "[process] Failed to open terminate event for pid {}: {}",
                    self.pid,
                    windows_sys::Win32::Foundation::GetLastError()
                );
            }
        }
        Ok(())
    }
}

unsafe impl Send for Process {}
unsafe impl Sync for Process {}
