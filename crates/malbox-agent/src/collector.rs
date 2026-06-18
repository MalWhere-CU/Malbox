use crate::bson_log::ApiCall;
use std::collections::{BTreeSet, HashMap};
pub struct ProcInfo {
    pub pid: u32,
    pub parent_id: u32,
    pub process_name: String,
    pub first_seen_order: usize,
    pub thread_ids: BTreeSet<u32>,
    pub calls: Vec<ApiCall>,
}
#[derive(Default)]
pub struct AnalysisCollector {
    pub procs: HashMap<u32, ProcInfo>,
    pub child_links: Vec<(u32, u32)>,
}
impl AnalysisCollector {
    pub fn register_process(&mut self, pid: u32, parent: u32, name: String) {
        let order = self.procs.len();
        self.procs.entry(pid).or_insert_with(|| ProcInfo {
            pid,
            parent_id: parent,
            process_name: name,
            first_seen_order: order,
            thread_ids: BTreeSet::new(),
            calls: Vec::new(),
        });
        if parent != 0 {
            self.child_links.push((parent, pid));
        }
    }
    pub fn push_call(&mut self, pid: u32, call: ApiCall) {
        if let Some(proc) = self.procs.get_mut(&pid) {
            proc.thread_ids.insert(call.thread_id);
            proc.calls.push(call);
        }
    }
    pub fn tracked_pids(&self) -> Vec<u32> {
        self.procs.keys().copied().collect()
    }
}
