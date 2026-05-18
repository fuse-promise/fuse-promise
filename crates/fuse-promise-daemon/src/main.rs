use fuse_promise_ipc::serve_status;
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
            println!("status=not-mounted");
            println!("fuse_adapter=not-implemented");
            if foreground {
                println!("mode=foreground");
            }

            let runtime = Arc::new(Mutex::new(Runtime::new()));
            match serve_status(runtime) {
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
    println!("The current skeleton serves private status IPC only; FUSE is not implemented.");
}
