use fuse_promise_ipc::{
    materialize_file_with_progress, query_inspect, query_status, MaterializeConflictPolicy,
    MaterializeRequest,
};
use fuse_promise_runtime::{default_control_socket_path, default_mount_path, CachePolicy};
use std::env;
use std::fs;
use std::io;
use std::path::PathBuf;
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
        Some("materialize") => {
            let mut conflict_policy = MaterializeConflictPolicy::Fail;
            let mut show_progress = false;
            let mut promise_path = None;
            for arg in args.by_ref() {
                match arg.as_str() {
                    "--overwrite" if conflict_policy == MaterializeConflictPolicy::Fail => {
                        conflict_policy = MaterializeConflictPolicy::Overwrite;
                    }
                    "--rename" if conflict_policy == MaterializeConflictPolicy::Fail => {
                        conflict_policy = MaterializeConflictPolicy::Rename;
                    }
                    "--overwrite" | "--rename" => {
                        eprintln!("fpctl: materialize conflict option was already set");
                        print_help();
                        return ExitCode::from(2);
                    }
                    "--progress" => {
                        show_progress = true;
                    }
                    option if option.starts_with("--") => {
                        eprintln!("fpctl: unknown materialize option: {option}");
                        print_help();
                        return ExitCode::from(2);
                    }
                    _ => {
                        promise_path = Some(arg);
                        break;
                    }
                }
            }
            let Some(promise_path) = promise_path else {
                eprintln!("fpctl: materialize requires a promise path");
                print_help();
                return ExitCode::from(2);
            };
            let Some(target_dir) = args.next() else {
                eprintln!("fpctl: materialize requires a target directory");
                print_help();
                return ExitCode::from(2);
            };
            if let Some(extra) = args.next() {
                eprintln!("fpctl: unexpected argument: {extra}");
                print_help();
                return ExitCode::from(2);
            }
            materialize(&promise_path, &target_dir, conflict_policy, show_progress)
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

fn materialize(
    promise_path: &str,
    target_dir: &str,
    conflict_policy: MaterializeConflictPolicy,
    show_progress: bool,
) -> ExitCode {
    let socket_path = match default_control_socket_path() {
        Ok(path) => path,
        Err(status) => {
            eprintln!("fpctl: {}", status.as_str());
            return ExitCode::from(1);
        }
    };
    let source_path = match absolute_client_path(promise_path) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("fpctl: {error}");
            return ExitCode::from(1);
        }
    };
    let target_dir = match checked_target_dir(target_dir) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("fpctl: {error}");
            return ExitCode::from(1);
        }
    };

    match materialize_file_with_progress(
        &socket_path,
        MaterializeRequest {
            source_path,
            target_dir,
            conflict_policy,
        },
        |progress| {
            if show_progress {
                eprintln!("{}", progress.encode_text());
            }
            Ok(())
        },
    ) {
        Ok(response) => {
            print!("{}", response.encode_text());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("fpctl: {error}");
            ExitCode::from(1)
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

fn absolute_client_path(path: &str) -> io::Result<PathBuf> {
    let path = PathBuf::from(path);
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(env::current_dir()?.join(path))
    }
}

fn checked_target_dir(path: &str) -> io::Result<PathBuf> {
    let path = PathBuf::from(path);
    let path = if path.is_absolute() {
        path
    } else {
        env::current_dir()?.join(path)
    };
    let metadata = fs::symlink_metadata(&path)?;
    if metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "target directory must not be a symlink",
        ));
    }
    if !metadata.is_dir() {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "target directory is not a directory",
        ))
    } else {
        Ok(path)
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
                println!("cache_policy={}", CachePolicy::NoCache.as_str());
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
    println!("  materialize [--progress] [--overwrite|--rename] <promise-path> <target-dir>");
}
