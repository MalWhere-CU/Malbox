use serde::Serialize;

use crate::collector::{AnalysisCollector, ProcInfo};

#[derive(Serialize)]
pub struct TargetFile {
    pub name: String,
    pub size: u64,
    #[serde(rename = "type")]
    pub r_type: String,
    pub cape_type_code: u32,
}

#[derive(Serialize)]
pub struct Target {
    pub file: TargetFile,
    pub category: String,
}

#[derive(Serialize, Default)]
pub struct Summary {
    pub files: Vec<serde_json::Value>,
    pub read_files: Vec<serde_json::Value>,
    pub write_files: Vec<serde_json::Value>,
    pub delete_files: Vec<serde_json::Value>,
}

#[derive(Serialize)]
pub struct CallEntry {
    pub api: String,
    pub category: String,
    pub status: String,
    pub return_value: i64,
    pub thread_id: u32,
    pub time: i32,
    pub arguments: Vec<Argument>,
}

#[derive(Serialize)]
pub struct Argument {
    pub name: String,
    pub value: String,
}

#[derive(Serialize)]
pub struct ProcessEntry {
    pub process_id: u32,
    pub parent_id: u32,
    pub process_name: String,
    pub threads: Vec<u32>,
    pub calls: Vec<CallEntry>,
}

#[derive(Serialize)]
pub struct TreeNode {
    pub pid: u32,
    pub name: String,
    pub children: Vec<TreeNode>,
}
#[derive(Serialize)]
pub struct Behavior {
    pub processes: Vec<ProcessEntry>,
    pub processtree: Vec<TreeNode>,
    pub summary: Summary,
    pub enhanced: Vec<serde_json::Value>,
    pub anomaly: Vec<serde_json::Value>,
    pub encryptedbuffers: Vec<serde_json::Value>,
}

#[derive(Serialize)]
pub struct DroppedEntry {
    pub size: u64,
}
#[derive(Serialize)]
pub struct Signature {
    pub name: String,
    pub alert: bool,
}
#[derive(Serialize)]
pub struct Cape {
    pub payloads: Vec<Payload>,
}
#[derive(Serialize)]
pub struct Payload {
    pub size: u64,
}

#[derive(Serialize)]
pub struct Report {
    pub target: Target,
    pub behavior: Behavior,
    pub dropped: Vec<DroppedEntry>,
    pub signatures: Vec<Signature>,
    #[serde(rename = "CAPE")]
    pub cape: Cape,
}

impl Report {
    pub fn build(collector: &AnalysisCollector, target: TargetFile, category: &str) -> Report {
        let mut procs: Vec<&ProcInfo> = collector.procs.values().collect();
        procs.sort_by_key(|p| p.first_seen_order);
        let processes: Vec<ProcessEntry> = procs
            .iter()
            .map(|p| ProcessEntry {
                process_id: p.pid,
                parent_id: p.parent_id,
                process_name: p.process_name.clone(),
                threads: p.thread_ids.iter().copied().collect(),
                calls: p
                    .calls
                    .iter()
                    .map(|c| CallEntry {
                        api: c.api.clone(),
                        category: c.category.clone(),
                        status: c.status.clone(),
                        return_value: c.return_value.clone(),
                        thread_id: c.thread_id,
                        time: c.time,
                        arguments: c
                            .arguments
                            .iter()
                            .map(|(n, v)| Argument {
                                name: n.clone(),
                                value: v.clone(),
                            })
                            .collect(),
                    })
                    .collect(),
            })
            .collect();
        let processtree = build_tree(&collector.procs, &collector.child_links);

        Report {
            target: Target {
                file: target,
                category: category.to_string(),
            },
            behavior: Behavior {
                processes,
                processtree,
                summary: Summary::default(),
                enhanced: vec![],
                anomaly: vec![],
                encryptedbuffers: vec![],
            },
            dropped: vec![],
            signatures: vec![],
            cape: Cape { payloads: vec![] },
        }
    }
}

fn build_tree(
    procs: &std::collections::HashMap<u32, ProcInfo>,
    child_links: &[(u32, u32)],
) -> Vec<TreeNode> {
    let mut children_map: std::collections::HashMap<u32, Vec<u32>> =
        std::collections::HashMap::new();
    for &(parent, child) in child_links {
        children_map.entry(parent).or_default().push(child);
    }
    let root_pids: Vec<u32> = procs
        .iter()
        .filter(|(_, p)| p.parent_id == 0 || !procs.contains_key(&p.parent_id))
        .map(|(pid, _)| *pid)
        .collect();
    fn build_node(
        pid: u32,
        procs: &std::collections::HashMap<u32, ProcInfo>,
        children_map: &std::collections::HashMap<u32, Vec<u32>>,
    ) -> TreeNode {
        let name = procs
            .get(&pid)
            .map(|p| p.process_name.clone())
            .unwrap_or_default();
        let children = children_map
            .get(&pid)
            .map(|v| {
                v.iter()
                    .map(|&c| build_node(c, procs, children_map))
                    .collect()
            })
            .unwrap_or_default();
        TreeNode {
            pid,
            name,
            children,
        }
    }
    root_pids
        .iter()
        .map(|&pid| build_node(pid, procs, &children_map))
        .collect()
}

pub fn build_target_file(path: &std::path::Path) -> anyhow::Result<TargetFile> {
    let meta = std::fs::metadata(path)?;
    let size = meta.len();
    let r_type = detect_pe_type(path).unwrap_or_else(|| "unknown".to_string());
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();
    Ok(TargetFile {
        name,
        size,
        r_type,
        cape_type_code: 0,
    })
}

fn detect_pe_type(path: &std::path::Path) -> Option<String> {
    let data = std::fs::read(path).ok()?;
    if data.len() < 0x40 {
        return None;
    }
    let pe_offset = u32::from_le_bytes([data[0x3c], data[0x3d], data[0x3e], data[0x3f]]) as usize;
    if pe_offset + 24 >= data.len() {
        return None;
    }
    let magic = u16::from_le_bytes([data[pe_offset + 24], data[pe_offset + 25]]);
    let is_dll = data.len() > pe_offset + 22
        && (u16::from_le_bytes([data[pe_offset + 22], data[pe_offset + 23]]) & 0x2000) != 0;
    let kind = if is_dll { "dll" } else { "exe" };
    let arch = match magic {
        0x10b => "pe32",
        0x20b => "pe32+",
        _ => return None,
    };
    Some(format!("{} {}", arch, kind))
}
pub fn report_to_json(r: &Report) -> anyhow::Result<String> {
    Ok(serde_json::to_string(r)?)
}
