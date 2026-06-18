use std::sync::{Arc, Mutex};

use crate::{
    bson_log::{SchemaRegistry, process_doc, read_bson_frame, read_header},
    collector::AnalysisCollector,
    pipes::{CommandCtx, raw::NamedPipe},
};

pub fn spawn_reader(ctx: &CommandCtx, pid: u32) -> anyhow::Result<()> {
    let pipe_name = format!("{}{}", ctx.state.log_pipe_prefix, pid);
    let pipe = NamedPipe::create_log(&pipe_name)?;
    println!("[log] Log pipe ready for pid {} on {}", pid, pipe_name);
    let collector = ctx.collector.clone();
    let handle = std::thread::spawn(move || read_loop(collector, pid, pipe));
    ctx.logserver_tasks.lock().unwrap().push(handle);
    Ok(())
}

fn read_loop(collector: Arc<Mutex<AnalysisCollector>>, pid: u32, mut server: NamedPipe) {
    if let Err(e) = server.connect() {
        eprintln!("[log] connect failed for pid {}: {}", pid, e);
        return;
    }
    let mut reg = SchemaRegistry::default();
    let (proto, pid) = match read_header(&mut server) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("[log] header read failed for pid {}: {}", pid, e);
            return;
        }
    };
    loop {
        match read_bson_frame(&mut server) {
            Ok(Some(doc)) => {
                if let Some(call) = process_doc(&mut reg, &doc) {
                    // println!("read a call: {:?}", call);
                    collector.lock().unwrap().push_call(pid, call);
                } else {
                    println!("[log] that was not a call");
                }
            }
            Ok(None) => {
                println!("[log] Pipe closed for pid {}", pid);
                break;
            }
            Err(e) => {
                eprintln!("[log] frame read error for pid {}: {}", pid, e);
                break;
            }
        }
    }
}
