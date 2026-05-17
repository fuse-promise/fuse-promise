use fuse_promise_runtime::default_mount_path;
use std::env;
use std::process::ExitCode;

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
            println!("status=not-mounted");
            println!("fuse_adapter=not-implemented");
            if foreground {
                eprintln!("fuse-promised: FUSE adapter is not implemented");
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
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
    println!("The FUSE adapter is not implemented in this initial skeleton.");
}
