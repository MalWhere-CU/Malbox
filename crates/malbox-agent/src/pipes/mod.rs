use std::sync::{Arc, Mutex, atomic::AtomicBool};
use std::thread::JoinHandle;

use crate::{collector::AnalysisCollector, monitor::MonitorState};

pub mod command;
pub mod logserver;
pub mod raw;

#[derive(Clone)]
pub struct CommandCtx {
    pub state: Arc<MonitorState>,
    pub collector: Arc<Mutex<AnalysisCollector>>,
    pub logserver_tasks: Arc<Mutex<Vec<JoinHandle<()>>>>,
    pub do_run: Arc<AtomicBool>,
}
