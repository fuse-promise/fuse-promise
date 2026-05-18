use fuse_promise_ipc::{query_inspect, query_status};
use fuse_promise_runtime::{default_control_socket_path, default_mount_path};
use std::env;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        None | Some("status") => {
            if let Some(extra) = args.next() {
                eprintln!("fpctl: unexpected argument: {extra}");
                print_help();
                return ExitCode::from(2);
            }
            status()
        }
        Some("list") => {
            if let Some(extra) = args.next() {
                eprintln!("fpctl: unexpected argument: {extra}");
                print_help();
                return ExitCode::from(2);
            }
            list()
        }
        Some("-h") | Some("--help") | Some("help") => {
            print_help();
            ExitCode::SUCCESS
        }
        Some(command) => {
            eprintln!("fpctl: unknown command: {command}");
            print_help();
            ExitCode::from(2)
        }
    }
}

fn list() -> ExitCode {
    let socket_path = match default_control_socket_path() {
        Ok(path) => path,
        Err(status) => {
            eprintln!("fpctl: {}", status.as_str());
            return ExitCode::from(1);
        }
    };

    match query_inspect(&socket_path) {
        Ok(response) => {
            print!("{response}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("fpctl: {error}");
            ExitCode::from(1)
        }
    }
}

fn status() -> ExitCode {
    let socket_path = match default_control_socket_path() {
        Ok(path) => path,
        Err(status) => {
            eprintln!("fpctl: {}", status.as_str());
            return ExitCode::from(1);
        }
    };

    match query_status(&socket_path) {
        Ok(response) => {
            print!("{response}");
            ExitCode::SUCCESS
        }
        Err(_) => match default_mount_path() {
            Ok(path) => {
                println!("mount_path={}", path.display());
                println!("socket_path={}", socket_path.display());
                println!("daemon=not-connected");
                println!("mount=not-mounted");
                println!("fuse_adapter=not-implemented");
                ExitCode::SUCCESS
            }
            Err(status) => {
                eprintln!("fpctl: {}", status.as_str());
                ExitCode::from(1)
            }
        },
    }
}

fn print_help() {
    println!("usage: fpctl <command>");
    println!();
    println!("commands:");
    println!("  status    Query the user-session daemon status");
    println!("  list      List daemon-owned promises and nodes");
}
