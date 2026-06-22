use std::{
    path::PathBuf,
    process::Command,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use std::ffi::CString;
use windows_sys::Win32::System::Threading::Sleep;

use crate::{
    collector::AnalysisCollector,
    monitor::MonitorState,
    pipes::{CommandCtx, command, logserver},
    process::Process,
    report::{self, Report},
};

pub struct AnalysisConfig {
    pub sample_path: PathBuf,
    pub timeout_secs: u32,
    pub work_dir: PathBuf,
}

pub struct AnalysisResult {
    pub exit_code: i32,
    pub runtime_ms: u64,
    pub features_json: String,
}

pub async fn run_analysis(config: AnalysisConfig) -> anyhow::Result<AnalysisResult> {
    let state = Arc::new(MonitorState::init(&config.work_dir)?);
    let collector = Arc::new(Mutex::new(AnalysisCollector::default()));
    let logserver_tasks = Arc::new(Mutex::new(Vec::<JoinHandle<()>>::new()));
    let do_run = Arc::new(AtomicBool::new(true));
    let ctx = CommandCtx {
        state: state.clone(),
        collector: collector.clone(),
        logserver_tasks: logserver_tasks.clone(),
        do_run: do_run.clone(),
    };

    let cmd_ctx = ctx.clone();
    let cmd_thread = std::thread::spawn(move || command::serve_command_pipe(cmd_ctx));
    let mut sample_process = Process {
        pid: 0,
        tid: 0,
        h_process: None,
        h_thread: None,
        suspended: true,
    };

    Command::new("net")
        .args(["stop", "winmgmt", "/y"])
        .status()
        .expect("failed to stop winmgmt");
    Command::new("sc.exe")
        .args(["config", "winmgmt", "type=", "own"])
        .status()
        .expect("failed to configure winmgmt");
    Command::new("net")
        .args(["start", "winmgmt"])
        .status()
        .expect("failed to start winmgmt");

    sample_process
        .execute(&config.sample_path, None, true)
        .unwrap();

    let sample_name = config
        .sample_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();
    collector
        .lock()
        .unwrap()
        .register_process(sample_process.pid, 0, sample_name);
    logserver::spawn_reader(&ctx, sample_process.pid)?;
    sample_process
        .inject(Some(config.sample_path.to_str().unwrap()), false, &state)
        .unwrap();
    sample_process.resume().unwrap();
    println!("[analysis] Sample resumed, monitoring...");
    sample_process.close().unwrap();
    let start = Instant::now();
    let timeout = Duration::from_secs(config.timeout_secs as u64);

    loop {
        tokio::time::sleep(Duration::from_millis(500)).await;
        if !sample_process.is_alive() {
            println!("[analysis] Sample exited");
            break;
        }
        if start.elapsed() >= timeout {
            println!("[analysis] Timeout reached, terminating sample");
            sample_process.terminate_and_close();
            break;
        }
    }

    let runtime_ms = start.elapsed().as_millis() as u64;
    signal_shutdown(&state).ok();
    drain_logservers(&logserver_tasks, Duration::from_secs(3)).await;
    do_run.store(false, Ordering::Relaxed);
    crate::pipes::raw::nudge_server(&state.command_pipe);
    let _ = cmd_thread.join();
    let target = report::build_target_file(&config.sample_path)?;
    let features_json = {
        let c = collector.lock().unwrap();
        let report = Report::build(&c, target, "file");
        report::report_to_json(&report)?
    };
    let exit_code = sample_process.exit_code().map(|c| c as i32).unwrap_or(-1);
    state.cleanup();
    Ok(AnalysisResult {
        exit_code,
        runtime_ms,
        features_json,
    })
}

fn signal_shutdown(state: &MonitorState) -> anyhow::Result<()> {
    let name = CString::new(state.shutdown_mutex.as_str())?;
    unsafe {
        let h = windows_sys::Win32::System::Threading::CreateMutexA(
            std::ptr::null(),
            windows_sys::Win32::Foundation::FALSE,
            name.as_ptr() as *const u8,
        );
        if !h.is_null() {
            println!("Created shutdown mutex");
        } else {
            return Err(anyhow::anyhow!("failed to create shutdown mutex"));
        }
    }
    unsafe {
        Sleep(1000);
    }
    Ok(())
}

async fn drain_logservers(tasks: &Arc<Mutex<Vec<JoinHandle<()>>>>, timeout: Duration) {
    let handles: Vec<_> = {
        let mut lock = tasks.lock().unwrap();
        std::mem::take(&mut *lock)
    };
    let join_all = tokio::task::spawn_blocking(move || {
        for h in handles {
            let _ = h.join();
        }
    });
    let _ = tokio::time::timeout(timeout, join_all).await;
}
