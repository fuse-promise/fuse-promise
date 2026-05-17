use fuse_promise_runtime::default_mount_path;
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

fn status() -> ExitCode {
    match default_mount_path() {
        Ok(path) => {
            println!("runtime_dir={}", path.display());
            println!("daemon=not-connected");
            println!("mount=not-mounted");
            ExitCode::SUCCESS
        }
        Err(status) => {
            eprintln!("fpctl: {}", status.as_str());
            ExitCode::from(1)
        }
    }
}

fn print_help() {
    println!("usage: fpctl <command>");
    println!();
    println!("commands:");
    println!("  status    Show the expected user-session runtime path");
}
