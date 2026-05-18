use fuse_promise_runtime::{
    default_control_socket_path, default_mount_path, Runtime, Status, API_VERSION,
};
use std::fmt::Write as _;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::FileTypeExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub const STATUS_COMMAND: &str = "STATUS";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonStatus {
    pub api_version: u32,
    pub mount_path: PathBuf,
    pub socket_path: PathBuf,
    pub daemon: &'static str,
    pub mount: &'static str,
    pub fuse_adapter: &'static str,
    pub providers: usize,
    pub promises: usize,
}

impl DaemonStatus {
    pub fn from_runtime(runtime: &Runtime) -> Result<Self, Status> {
        Ok(Self {
            api_version: API_VERSION,
            mount_path: default_mount_path()?,
            socket_path: default_control_socket_path()?,
            daemon: "connected",
            mount: "not-mounted",
            fuse_adapter: "not-implemented",
            providers: runtime.provider_count(),
            promises: runtime.promise_count(),
        })
    }

    pub fn encode(&self) -> String {
        let mut output = String::new();
        let _ = writeln!(output, "ok");
        let _ = writeln!(output, "api_version={}", self.api_version);
        let _ = writeln!(output, "mount_path={}", self.mount_path.display());
        let _ = writeln!(output, "socket_path={}", self.socket_path.display());
        let _ = writeln!(output, "daemon={}", self.daemon);
        let _ = writeln!(output, "mount={}", self.mount);
        let _ = writeln!(output, "fuse_adapter={}", self.fuse_adapter);
        let _ = writeln!(output, "providers={}", self.providers);
        let _ = writeln!(output, "promises={}", self.promises);
        output
    }
}

pub fn query_status(socket_path: &Path) -> std::io::Result<String> {
    let mut stream = UnixStream::connect(socket_path)?;
    stream.write_all(STATUS_COMMAND.as_bytes())?;
    stream.write_all(b"\n")?;
    stream.shutdown(std::net::Shutdown::Write)?;

    let mut response = String::new();
    BufReader::new(stream).read_to_string(&mut response)?;
    Ok(response)
}

pub fn serve_status(runtime: Arc<Mutex<Runtime>>) -> std::io::Result<()> {
    let socket_path = default_control_socket_path().map_err(status_to_io)?;
    bind_status_socket(&socket_path, runtime)
}

fn bind_status_socket(socket_path: &Path, runtime: Arc<Mutex<Runtime>>) -> std::io::Result<()> {
    if let Some(parent) = socket_path.parent() {
        fs::create_dir_all(parent)?;
    }
    remove_stale_socket(socket_path)?;

    let listener = UnixListener::bind(socket_path)?;
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => handle_client(stream, &runtime)?,
            Err(error) => return Err(error),
        }
    }

    Ok(())
}

fn handle_client(stream: UnixStream, runtime: &Arc<Mutex<Runtime>>) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream);
    let mut command = String::new();
    reader.read_line(&mut command)?;

    let mut stream = reader.into_inner();
    match command.trim_end() {
        STATUS_COMMAND => {
            let runtime = runtime
                .lock()
                .map_err(|_| std::io::Error::other("runtime lock poisoned"))?;
            let status = DaemonStatus::from_runtime(&runtime).map_err(status_to_io)?;
            stream.write_all(status.encode().as_bytes())?;
        }
        _ => {
            stream.write_all(b"error=unknown-command\n")?;
        }
    }

    Ok(())
}

fn remove_stale_socket(socket_path: &Path) -> std::io::Result<()> {
    let Ok(metadata) = fs::symlink_metadata(socket_path) else {
        return Ok(());
    };
    if !metadata.file_type().is_socket() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "control socket path exists and is not a socket",
        ));
    }

    match fs::remove_file(socket_path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn status_to_io(status: Status) -> std::io::Error {
    std::io::Error::other(status.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_encoding_is_line_based() {
        let status = DaemonStatus {
            api_version: 1,
            mount_path: PathBuf::from("/run/user/1000/fuse-promise"),
            socket_path: PathBuf::from("/run/user/1000/fuse-promise.sock"),
            daemon: "connected",
            mount: "not-mounted",
            fuse_adapter: "not-implemented",
            providers: 2,
            promises: 3,
        };

        let encoded = status.encode();
        assert!(encoded.starts_with("ok\n"));
        assert!(encoded.contains("api_version=1\n"));
        assert!(encoded.contains("providers=2\n"));
        assert!(encoded.contains("promises=3\n"));
    }
}
