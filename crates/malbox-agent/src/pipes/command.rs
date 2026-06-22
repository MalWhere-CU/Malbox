use std::io::{Read, Write};
use std::sync::atomic::Ordering;

use crate::pipes::{CommandCtx, raw::BUFSIZE, raw::NamedPipe};

/// Accept loop for the command pipe, mirroring CAPE's `PipeServer.run` with
/// `message=True`. Each accepted connection is handed to a dedicated thread so
/// the loop can immediately create and wait on the next instance.
///
/// Runs on its own blocking thread (see `analysis::run_analysis`). It exits
/// once `ctx.do_run` is cleared and one more client connects to release the
/// pending `ConnectNamedPipe` (the shutdown path opens such a client).
pub fn serve_command_pipe(ctx: CommandCtx) {
    let pipe_name = ctx.state.command_pipe.clone();
    println!("[cmd] Command pipe listening on {}", pipe_name);
    while ctx.do_run.load(Ordering::Relaxed) {
        let pipe = match NamedPipe::create_command(&pipe_name) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("[cmd] failed to create pipe: {}", e);
                break;
            }
        };
        if let Err(e) = pipe.connect() {
            eprintln!("[cmd] connect error: {}", e);
            continue;
        }
        // Re-check after waking from the blocking connect: the shutdown path
        // sets do_run=false then connects a throwaway client to unblock us.
        if !ctx.do_run.load(Ordering::Relaxed) {
            break;
        }
        let ctx = ctx.clone();
        std::thread::spawn(move || handle_command_connection(ctx, pipe));
    }
}

fn handle_command_connection(_ctx: CommandCtx, mut pipe: NamedPipe) {
    let mut buf = vec![0u8; BUFSIZE as usize];
    loop {
        match pipe.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                println!("[cmd] {}", String::from_utf8_lossy(&buf[..n]));
                // capemon writes a command then reads a response; reply so it
                // does not stall (PipeDispatcher defaults to b"OK").
                let _ = pipe.write_all(b"OK");
            }
            Err(_) => break,
        }
    }
    pipe.disconnect();
}
