mod fuse_adapter;

use fuse_promise_ipc::{serve_state, IpcState};
use fuse_promise_runtime::{default_control_socket_path, default_mount_path, Runtime};
use std::env;
use std::process::ExitCode;
use std::sync::{Arc, Mutex};

fn main() -> ExitCode {
    let mut foreground = false;
    for arg in env::args().skip(1) {
        match arg.as_str() {
            "-h" | "--help" => {
                print_help();
                return ExitCode::SUCCESS;
            }
            "--foreground" => foreground = true,
            _ => {
                eprintln!("fuse-promised: unknown argument: {arg}");
                print_help();
                return ExitCode::from(2);
            }
        }
    }

    match default_mount_path() {
        Ok(path) => {
            println!("fuse-promised");
            println!("mount_path={}", path.display());
            match default_control_socket_path() {
                Ok(socket_path) => println!("socket_path={}", socket_path.display()),
                Err(status) => {
                    eprintln!("fuse-promised: {}", status.as_str());
                    return ExitCode::from(1);
                }
            }
            if foreground {
                println!("mode=foreground");
            }

            let runtime = Arc::new(Mutex::new(Runtime::new()));
            let ipc_state = IpcState::new(Arc::clone(&runtime));
            let fuse_mount = match fuse_adapter::start(&path, ipc_state.clone()) {
                Ok(mount) => mount,
                Err(error) => {
                    eprintln!("fuse-promised: failed to mount {}: {error}", path.display());
                    return ExitCode::from(1);
                }
            };
            let mount_status = fuse_adapter::mount_status(&path, &fuse_mount);
            if let Err(error) = ipc_state.set_mount_status(mount_status.clone()) {
                eprintln!("fuse-promised: {error}");
                return ExitCode::from(1);
            }
            println!("mount={}", mount_status.mount);
            println!("fuse_adapter={}", mount_status.fuse_adapter);

            match serve_state(ipc_state) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("fuse-promised: {error}");
                    ExitCode::from(1)
                }
            }
        }
        Err(status) => {
            eprintln!("fuse-promised: {}", status.as_str());
            ExitCode::from(1)
        }
    }
}

fn print_help() {
    println!("usage: fuse-promised [--foreground]");
    println!();
    println!("Starts the user-session Promise filesystem daemon.");
    println!("FUSE mounting requires building the daemon with the fuse-mount feature.");
}
