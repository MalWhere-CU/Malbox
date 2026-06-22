use std::io::{Read, Write};
use std::sync::atomic::Ordering;

use crate::pipes::{CommandCtx, raw::BUFSIZE, raw::NamedPipe};

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
                let _ = pipe.write_all(b"OK");
            }
            Err(_) => break,
        }
    }
    pipe.disconnect();
}
