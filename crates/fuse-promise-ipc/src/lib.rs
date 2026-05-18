use bincode::{Decode, Encode};
use fuse_promise_runtime::{
    default_control_socket_path, default_mount_path, NodeAttr, PromiseBuilder, Runtime, Status,
    API_VERSION,
};
use std::fmt::Write as _;
use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::fs::FileTypeExt;
use std::os::unix::net::UnixListener;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub const IPC_PROTOCOL_VERSION: u32 = 1;
pub const MAX_FRAME_LEN: u32 = 1024 * 1024;

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
        StatusBody::from_status(self).encode_text()
    }
}

#[derive(Debug, Clone, Encode, Decode, PartialEq, Eq)]
enum Request {
    Hello {
        protocol_version: u32,
        api_version: u32,
    },
    Status,
    ProviderRegister,
    ProviderUnregister {
        provider_id: u64,
    },
    PromiseCommit(PromiseCommitBody),
}

#[derive(Debug, Clone, Encode, Decode, PartialEq, Eq)]
enum Response {
    Hello {
        protocol_version: u32,
        api_version: u32,
    },
    Status(StatusBody),
    ProviderRegistered {
        provider_id: u64,
    },
    ProviderUnregistered,
    PromiseCommitted(PromiseCommittedBody),
    Error(ErrorBody),
}

#[derive(Debug, Clone, Encode, Decode, PartialEq, Eq)]
struct StatusBody {
    api_version: u32,
    mount_path: String,
    socket_path: String,
    daemon: String,
    mount: String,
    fuse_adapter: String,
    providers: u64,
    promises: u64,
}

#[derive(Debug, Clone, Encode, Decode, PartialEq, Eq)]
struct PromiseCommitBody {
    provider_id: u64,
    nodes: Vec<PromiseNodeBody>,
}

#[derive(Debug, Clone, Encode, Decode, PartialEq, Eq)]
struct PromiseNodeBody {
    kind: PromiseNodeKindBody,
    relative_path: String,
    provider_node_id: String,
    mode: u32,
    size: u64,
    mtime_nsec: i64,
}

#[derive(Debug, Clone, Copy, Encode, Decode, PartialEq, Eq)]
enum PromiseNodeKindBody {
    File,
    Directory,
}

#[derive(Debug, Clone, Encode, Decode, PartialEq, Eq)]
struct PromiseCommittedBody {
    promise_id: String,
}

impl StatusBody {
    fn from_status(status: &DaemonStatus) -> Self {
        Self {
            api_version: status.api_version,
            mount_path: status.mount_path.to_string_lossy().into_owned(),
            socket_path: status.socket_path.to_string_lossy().into_owned(),
            daemon: status.daemon.to_owned(),
            mount: status.mount.to_owned(),
            fuse_adapter: status.fuse_adapter.to_owned(),
            providers: status.providers as u64,
            promises: status.promises as u64,
        }
    }

    fn encode_text(&self) -> String {
        let mut output = String::new();
        let _ = writeln!(output, "ok");
        let _ = writeln!(output, "api_version={}", self.api_version);
        let _ = writeln!(output, "mount_path={}", self.mount_path);
        let _ = writeln!(output, "socket_path={}", self.socket_path);
        let _ = writeln!(output, "daemon={}", self.daemon);
        let _ = writeln!(output, "mount={}", self.mount);
        let _ = writeln!(output, "fuse_adapter={}", self.fuse_adapter);
        let _ = writeln!(output, "providers={}", self.providers);
        let _ = writeln!(output, "promises={}", self.promises);
        output
    }
}

#[derive(Debug, Clone, Encode, Decode, PartialEq, Eq)]
struct ErrorBody {
    code: ErrorCode,
    message: String,
}

#[derive(Debug, Clone, Copy, Encode, Decode, PartialEq, Eq)]
enum ErrorCode {
    InvalidRequest,
    VersionMismatch,
    NotFound,
    ProviderGone,
    Permission,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderRegistration {
    pub provider_id: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromiseCommitRequest {
    pub provider_id: u64,
    pub nodes: Vec<PromiseNodeSpec>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromiseNodeSpec {
    pub kind: PromiseNodeKind,
    pub relative_path: String,
    pub provider_node_id: String,
    pub attr: PromiseNodeAttr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromiseNodeKind {
    File,
    Directory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PromiseNodeAttr {
    pub mode: u32,
    pub size: u64,
    pub mtime_nsec: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromiseCommitResponse {
    pub promise_id: String,
}

pub fn query_status(socket_path: &Path) -> io::Result<String> {
    let mut stream = connect_and_hello(socket_path)?;

    write_frame(&mut stream, &Request::Status)?;
    match read_response(&mut stream)? {
        Response::Status(status) => Ok(status.encode_text()),
        Response::Error(error) => Err(error_to_io(error)),
        _ => Err(invalid_data(
            "daemon returned an unexpected status response",
        )),
    }
}

pub fn register_provider(socket_path: &Path) -> io::Result<ProviderRegistration> {
    let mut stream = connect_and_hello(socket_path)?;

    write_frame(&mut stream, &Request::ProviderRegister)?;
    match read_response(&mut stream)? {
        Response::ProviderRegistered { provider_id } => Ok(ProviderRegistration { provider_id }),
        Response::Error(error) => Err(error_to_io(error)),
        _ => Err(invalid_data(
            "daemon returned an unexpected provider register response",
        )),
    }
}

pub fn unregister_provider(socket_path: &Path, provider_id: u64) -> io::Result<()> {
    let mut stream = connect_and_hello(socket_path)?;

    write_frame(&mut stream, &Request::ProviderUnregister { provider_id })?;
    match read_response(&mut stream)? {
        Response::ProviderUnregistered => Ok(()),
        Response::Error(error) => Err(error_to_io(error)),
        _ => Err(invalid_data(
            "daemon returned an unexpected provider unregister response",
        )),
    }
}

pub fn commit_promise(
    socket_path: &Path,
    request: PromiseCommitRequest,
) -> io::Result<PromiseCommitResponse> {
    let mut stream = connect_and_hello(socket_path)?;

    write_frame(&mut stream, &Request::PromiseCommit(request.into_body()))?;
    match read_response(&mut stream)? {
        Response::PromiseCommitted(response) => Ok(PromiseCommitResponse {
            promise_id: response.promise_id,
        }),
        Response::Error(error) => Err(error_to_io(error)),
        _ => Err(invalid_data(
            "daemon returned an unexpected promise commit response",
        )),
    }
}

fn connect_and_hello(socket_path: &Path) -> io::Result<UnixStream> {
    let mut stream = UnixStream::connect(socket_path)?;
    write_frame(
        &mut stream,
        &Request::Hello {
            protocol_version: IPC_PROTOCOL_VERSION,
            api_version: API_VERSION,
        },
    )?;
    expect_hello(read_response(&mut stream)?)?;
    Ok(stream)
}

pub fn serve_status(runtime: Arc<Mutex<Runtime>>) -> io::Result<()> {
    let socket_path = default_control_socket_path().map_err(status_to_io)?;
    bind_status_socket(&socket_path, runtime)
}

fn bind_status_socket(socket_path: &Path, runtime: Arc<Mutex<Runtime>>) -> io::Result<()> {
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

fn handle_client(mut stream: UnixStream, runtime: &Arc<Mutex<Runtime>>) -> io::Result<()> {
    validate_peer(&stream)?;

    let mut negotiated = false;
    while let Some(request) = read_frame::<_, Request>(&mut stream)? {
        match request {
            Request::Hello {
                protocol_version,
                api_version,
            } => {
                if protocol_version == IPC_PROTOCOL_VERSION && api_version == API_VERSION {
                    negotiated = true;
                    write_frame(
                        &mut stream,
                        &Response::Hello {
                            protocol_version: IPC_PROTOCOL_VERSION,
                            api_version: API_VERSION,
                        },
                    )?;
                } else {
                    write_error(
                        &mut stream,
                        ErrorCode::VersionMismatch,
                        "unsupported IPC protocol or API version",
                    )?;
                }
            }
            Request::Status if negotiated => {
                let runtime = runtime
                    .lock()
                    .map_err(|_| io::Error::other("runtime lock poisoned"))?;
                let status = DaemonStatus::from_runtime(&runtime).map_err(status_to_io)?;
                write_frame(
                    &mut stream,
                    &Response::Status(StatusBody::from_status(&status)),
                )?;
            }
            Request::Status => {
                write_error(
                    &mut stream,
                    ErrorCode::InvalidRequest,
                    "client must send hello before status",
                )?;
            }
            Request::ProviderRegister if negotiated => {
                let mut runtime = runtime
                    .lock()
                    .map_err(|_| io::Error::other("runtime lock poisoned"))?;
                let provider_id = runtime.register_provider().raw();
                write_frame(&mut stream, &Response::ProviderRegistered { provider_id })?;
            }
            Request::ProviderRegister => {
                write_error(
                    &mut stream,
                    ErrorCode::InvalidRequest,
                    "client must send hello before provider register",
                )?;
            }
            Request::ProviderUnregister { provider_id } if negotiated => {
                let Some(provider_id) = fuse_promise_runtime::ProviderId::from_raw(provider_id)
                else {
                    write_error(
                        &mut stream,
                        ErrorCode::InvalidRequest,
                        "provider id must be nonzero",
                    )?;
                    continue;
                };

                let mut runtime = runtime
                    .lock()
                    .map_err(|_| io::Error::other("runtime lock poisoned"))?;
                match runtime.unregister_provider(provider_id) {
                    Ok(()) => write_frame(&mut stream, &Response::ProviderUnregistered)?,
                    Err(status) => write_status_error(&mut stream, status)?,
                }
            }
            Request::ProviderUnregister { .. } => {
                write_error(
                    &mut stream,
                    ErrorCode::InvalidRequest,
                    "client must send hello before provider unregister",
                )?;
            }
            Request::PromiseCommit(request) if negotiated => {
                handle_promise_commit(&mut stream, runtime, request)?;
            }
            Request::PromiseCommit(_) => {
                write_error(
                    &mut stream,
                    ErrorCode::InvalidRequest,
                    "client must send hello before promise commit",
                )?;
            }
        }
    }

    Ok(())
}

fn handle_promise_commit(
    stream: &mut UnixStream,
    runtime: &Arc<Mutex<Runtime>>,
    request: PromiseCommitBody,
) -> io::Result<()> {
    let Some(provider_id) = fuse_promise_runtime::ProviderId::from_raw(request.provider_id) else {
        write_error(
            stream,
            ErrorCode::InvalidRequest,
            "provider id must be nonzero",
        )?;
        return Ok(());
    };

    let mut builder = PromiseBuilder::new(provider_id);
    for node in request.nodes {
        let attr = NodeAttr::new(node.mode, node.size, node.mtime_nsec);
        let result = match node.kind {
            PromiseNodeKindBody::File => {
                builder.add_file(&node.relative_path, attr, &node.provider_node_id)
            }
            PromiseNodeKindBody::Directory => {
                builder.add_dir(&node.relative_path, attr, &node.provider_node_id)
            }
        };

        if let Err(status) = result {
            write_status_error(stream, status)?;
            return Ok(());
        }
    }

    let tree = {
        let mut runtime = runtime
            .lock()
            .map_err(|_| io::Error::other("runtime lock poisoned"))?;
        match runtime.commit_promise(builder) {
            Ok(tree) => tree,
            Err(status) => {
                write_status_error(stream, status)?;
                return Ok(());
            }
        }
    };

    write_frame(
        stream,
        &Response::PromiseCommitted(PromiseCommittedBody {
            promise_id: tree.promise_id,
        }),
    )
}

fn read_response<R>(reader: &mut R) -> io::Result<Response>
where
    R: Read,
{
    read_frame(reader)?.ok_or_else(|| io::Error::from(io::ErrorKind::UnexpectedEof))
}

fn expect_hello(response: Response) -> io::Result<()> {
    match response {
        Response::Hello {
            protocol_version,
            api_version,
        } if protocol_version == IPC_PROTOCOL_VERSION && api_version == API_VERSION => Ok(()),
        Response::Hello { .. } => Err(invalid_data(
            "daemon returned an incompatible hello response",
        )),
        Response::Error(error) => Err(error_to_io(error)),
        _ => Err(invalid_data("daemon returned an unexpected hello response")),
    }
}

fn write_error<W>(writer: &mut W, code: ErrorCode, message: &str) -> io::Result<()>
where
    W: Write,
{
    write_frame(
        writer,
        &Response::Error(ErrorBody {
            code,
            message: message.to_owned(),
        }),
    )
}

fn write_status_error<W>(writer: &mut W, status: Status) -> io::Result<()>
where
    W: Write,
{
    write_error(writer, status_to_error_code(status), status.as_str())
}

fn read_frame<R, T>(reader: &mut R) -> io::Result<Option<T>>
where
    R: Read,
    T: Decode<()>,
{
    let mut first = [0_u8; 1];
    match reader.read(&mut first)? {
        0 => return Ok(None),
        1 => {}
        _ => unreachable!(),
    }

    let mut rest = [0_u8; 3];
    reader.read_exact(&mut rest)?;
    let len = u32::from_le_bytes([first[0], rest[0], rest[1], rest[2]]) as usize;
    if len > MAX_FRAME_LEN as usize {
        return Err(invalid_data("IPC frame exceeds maximum length"));
    }

    let mut body = vec![0_u8; len];
    reader.read_exact(&mut body)?;
    let (message, bytes_read): (T, usize) =
        bincode::decode_from_slice(&body, bincode::config::standard())
            .map_err(decode_error_to_io)?;
    if bytes_read != body.len() {
        return Err(invalid_data("IPC frame has trailing bytes"));
    }

    Ok(Some(message))
}

fn write_frame<W, T>(writer: &mut W, message: &T) -> io::Result<()>
where
    W: Write,
    T: Encode,
{
    let body =
        bincode::encode_to_vec(message, bincode::config::standard()).map_err(encode_error_to_io)?;
    if body.len() > MAX_FRAME_LEN as usize {
        return Err(invalid_data("IPC frame exceeds maximum length"));
    }

    let len = body.len() as u32;
    writer.write_all(&len.to_le_bytes())?;
    writer.write_all(&body)?;
    writer.flush()
}

fn validate_peer(stream: &UnixStream) -> io::Result<()> {
    let peer = rustix::net::sockopt::socket_peercred(stream)?;
    let current_uid = rustix::process::getuid().as_raw();
    if peer.uid.as_raw() != current_uid {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "IPC peer uid does not match current user",
        ));
    }

    Ok(())
}

fn remove_stale_socket(socket_path: &Path) -> io::Result<()> {
    let Ok(metadata) = fs::symlink_metadata(socket_path) else {
        return Ok(());
    };
    if !metadata.file_type().is_socket() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "control socket path exists and is not a socket",
        ));
    }

    match fs::remove_file(socket_path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn error_to_io(error: ErrorBody) -> io::Error {
    let kind = match error.code {
        ErrorCode::InvalidRequest | ErrorCode::VersionMismatch => io::ErrorKind::InvalidData,
        ErrorCode::NotFound | ErrorCode::ProviderGone => io::ErrorKind::NotFound,
        ErrorCode::Permission => io::ErrorKind::PermissionDenied,
        ErrorCode::Internal => io::ErrorKind::Other,
    };

    io::Error::new(kind, error.message)
}

fn status_to_error_code(status: Status) -> ErrorCode {
    match status {
        Status::InvalidArgument => ErrorCode::InvalidRequest,
        Status::Permission => ErrorCode::Permission,
        Status::NotFound => ErrorCode::NotFound,
        Status::ProviderGone => ErrorCode::ProviderGone,
        Status::VersionMismatch => ErrorCode::VersionMismatch,
        _ => ErrorCode::Internal,
    }
}

impl PromiseCommitRequest {
    fn into_body(self) -> PromiseCommitBody {
        PromiseCommitBody {
            provider_id: self.provider_id,
            nodes: self
                .nodes
                .into_iter()
                .map(PromiseNodeSpec::into_body)
                .collect(),
        }
    }
}

impl PromiseNodeSpec {
    fn into_body(self) -> PromiseNodeBody {
        PromiseNodeBody {
            kind: match self.kind {
                PromiseNodeKind::File => PromiseNodeKindBody::File,
                PromiseNodeKind::Directory => PromiseNodeKindBody::Directory,
            },
            relative_path: self.relative_path,
            provider_node_id: self.provider_node_id,
            mode: self.attr.mode,
            size: self.attr.size,
            mtime_nsec: self.attr.mtime_nsec,
        }
    }
}

fn encode_error_to_io(error: bincode::error::EncodeError) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}

fn decode_error_to_io(error: bincode::error::DecodeError) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}

fn invalid_data(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn status_to_io(status: Status) -> io::Error {
    io::Error::other(status.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use fuse_promise_runtime::ProviderState;
    use std::io::Cursor;
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn status_encoding_is_key_value_text() {
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

    #[test]
    fn framed_response_round_trips() {
        let body = StatusBody {
            api_version: 1,
            mount_path: "/run/user/1000/fuse-promise".to_owned(),
            socket_path: "/run/user/1000/fuse-promise.sock".to_owned(),
            daemon: "connected".to_owned(),
            mount: "not-mounted".to_owned(),
            fuse_adapter: "not-implemented".to_owned(),
            providers: 2,
            promises: 3,
        };

        let mut frame = Vec::new();
        write_frame(&mut frame, &Response::Status(body.clone())).unwrap();
        let decoded: Response = read_frame(&mut Cursor::new(frame)).unwrap().unwrap();

        assert_eq!(decoded, Response::Status(body));
    }

    #[test]
    fn read_frame_rejects_oversize_payload() {
        let mut frame = Vec::new();
        frame.extend_from_slice(&(MAX_FRAME_LEN + 1).to_le_bytes());

        let error = read_frame::<_, Request>(&mut Cursor::new(frame)).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn client_and_server_negotiate_status() {
        std::env::set_var("XDG_RUNTIME_DIR", "/tmp");
        let (mut client, server) = UnixStream::pair().unwrap();
        let mut runtime = Runtime::new();
        runtime.register_provider();
        let runtime = Arc::new(Mutex::new(runtime));
        let server_runtime = Arc::clone(&runtime);
        let server_thread = thread::spawn(move || handle_client(server, &server_runtime));

        write_frame(
            &mut client,
            &Request::Hello {
                protocol_version: IPC_PROTOCOL_VERSION,
                api_version: API_VERSION,
            },
        )
        .unwrap();
        let hello: Response = read_frame(&mut client).unwrap().unwrap();
        assert_eq!(
            hello,
            Response::Hello {
                protocol_version: IPC_PROTOCOL_VERSION,
                api_version: API_VERSION,
            }
        );

        write_frame(&mut client, &Request::Status).unwrap();
        let response: Response = read_frame(&mut client).unwrap().unwrap();
        match response {
            Response::Status(status) => {
                assert_eq!(status.providers, 1);
                assert_eq!(status.promises, 0);
                assert_eq!(status.daemon, "connected");
            }
            other => panic!("unexpected response: {other:?}"),
        }

        drop(client);
        server_thread.join().unwrap().unwrap();
    }

    #[test]
    fn server_rejects_bad_hello_version() {
        let (mut client, server) = UnixStream::pair().unwrap();
        let runtime = Arc::new(Mutex::new(Runtime::new()));
        let server_runtime = Arc::clone(&runtime);
        let server_thread = thread::spawn(move || handle_client(server, &server_runtime));

        write_frame(
            &mut client,
            &Request::Hello {
                protocol_version: IPC_PROTOCOL_VERSION + 1,
                api_version: API_VERSION,
            },
        )
        .unwrap();
        let response: Response = read_frame(&mut client).unwrap().unwrap();

        assert_eq!(
            response,
            Response::Error(ErrorBody {
                code: ErrorCode::VersionMismatch,
                message: "unsupported IPC protocol or API version".to_owned(),
            })
        );

        drop(client);
        server_thread.join().unwrap().unwrap();
    }

    #[test]
    fn server_rejects_status_before_hello() {
        let (mut client, server) = UnixStream::pair().unwrap();
        let runtime = Arc::new(Mutex::new(Runtime::new()));
        let server_runtime = Arc::clone(&runtime);
        let server_thread = thread::spawn(move || handle_client(server, &server_runtime));

        write_frame(&mut client, &Request::Status).unwrap();
        let response: Response = read_frame(&mut client).unwrap().unwrap();

        assert_eq!(
            response,
            Response::Error(ErrorBody {
                code: ErrorCode::InvalidRequest,
                message: "client must send hello before status".to_owned(),
            })
        );

        drop(client);
        server_thread.join().unwrap().unwrap();
    }

    #[test]
    fn provider_register_and_unregister_mutate_runtime() {
        let (mut client, server) = UnixStream::pair().unwrap();
        let runtime = Arc::new(Mutex::new(Runtime::new()));
        let server_runtime = Arc::clone(&runtime);
        let server_thread = thread::spawn(move || handle_client(server, &server_runtime));

        send_hello(&mut client);

        write_frame(&mut client, &Request::ProviderRegister).unwrap();
        let response: Response = read_frame(&mut client).unwrap().unwrap();
        let provider_id = match response {
            Response::ProviderRegistered { provider_id } => provider_id,
            other => panic!("unexpected response: {other:?}"),
        };
        assert_eq!(provider_id, 1);

        write_frame(&mut client, &Request::ProviderUnregister { provider_id }).unwrap();
        let response: Response = read_frame(&mut client).unwrap().unwrap();
        assert_eq!(response, Response::ProviderUnregistered);

        drop(client);
        server_thread.join().unwrap().unwrap();

        let provider_id = fuse_promise_runtime::ProviderId::from_raw(provider_id).unwrap();
        assert_eq!(
            runtime.lock().unwrap().provider(provider_id).unwrap().state,
            ProviderState::Disconnected
        );
    }

    #[test]
    fn provider_unregister_rejects_unknown_provider() {
        let (mut client, server) = UnixStream::pair().unwrap();
        let runtime = Arc::new(Mutex::new(Runtime::new()));
        let server_runtime = Arc::clone(&runtime);
        let server_thread = thread::spawn(move || handle_client(server, &server_runtime));

        send_hello(&mut client);
        write_frame(
            &mut client,
            &Request::ProviderUnregister { provider_id: 99 },
        )
        .unwrap();
        let response: Response = read_frame(&mut client).unwrap().unwrap();

        assert_eq!(
            response,
            Response::Error(ErrorBody {
                code: ErrorCode::NotFound,
                message: "not found".to_owned(),
            })
        );

        drop(client);
        server_thread.join().unwrap().unwrap();
    }

    #[test]
    fn promise_commit_mutates_runtime() {
        let (mut client, server) = UnixStream::pair().unwrap();
        let runtime = Arc::new(Mutex::new(Runtime::new()));
        let server_runtime = Arc::clone(&runtime);
        let server_thread = thread::spawn(move || handle_client(server, &server_runtime));

        send_hello(&mut client);
        write_frame(&mut client, &Request::ProviderRegister).unwrap();
        let provider_id = match read_frame(&mut client).unwrap().unwrap() {
            Response::ProviderRegistered { provider_id } => provider_id,
            other => panic!("unexpected response: {other:?}"),
        };

        write_frame(
            &mut client,
            &Request::PromiseCommit(sample_commit_request(provider_id).into_body()),
        )
        .unwrap();
        let response: Response = read_frame(&mut client).unwrap().unwrap();
        assert_eq!(
            response,
            Response::PromiseCommitted(PromiseCommittedBody {
                promise_id: "promise-1".to_owned(),
            })
        );

        drop(client);
        server_thread.join().unwrap().unwrap();

        let runtime = runtime.lock().unwrap();
        let tree = runtime.promise("promise-1").unwrap();
        assert!(tree.get("docs/readme.txt").is_some());
        assert_eq!(runtime.promise_count(), 1);
    }

    #[test]
    fn promise_commit_rejects_unknown_provider() {
        let (mut client, server) = UnixStream::pair().unwrap();
        let runtime = Arc::new(Mutex::new(Runtime::new()));
        let server_runtime = Arc::clone(&runtime);
        let server_thread = thread::spawn(move || handle_client(server, &server_runtime));

        send_hello(&mut client);
        write_frame(
            &mut client,
            &Request::PromiseCommit(sample_commit_request(99).into_body()),
        )
        .unwrap();
        let response: Response = read_frame(&mut client).unwrap().unwrap();

        assert_eq!(
            response,
            Response::Error(ErrorBody {
                code: ErrorCode::ProviderGone,
                message: "provider gone".to_owned(),
            })
        );

        drop(client);
        server_thread.join().unwrap().unwrap();
        assert_eq!(runtime.lock().unwrap().promise_count(), 0);
    }

    #[test]
    fn provider_helpers_use_unix_socket() {
        let socket_path = unique_socket_path();
        let listener = UnixListener::bind(&socket_path).unwrap();
        let runtime = Arc::new(Mutex::new(Runtime::new()));
        let server_runtime = Arc::clone(&runtime);
        let server_thread = thread::spawn(move || {
            for _ in 0..2 {
                let (stream, _) = listener.accept().unwrap();
                handle_client(stream, &server_runtime).unwrap();
            }
        });

        let registration = register_provider(&socket_path).unwrap();
        unregister_provider(&socket_path, registration.provider_id).unwrap();

        server_thread.join().unwrap();
        let provider_id =
            fuse_promise_runtime::ProviderId::from_raw(registration.provider_id).unwrap();
        assert_eq!(
            runtime.lock().unwrap().provider(provider_id).unwrap().state,
            ProviderState::Disconnected
        );

        let _ = fs::remove_file(socket_path);
    }

    #[test]
    fn commit_helper_uses_unix_socket() {
        let socket_path = unique_socket_path();
        let listener = UnixListener::bind(&socket_path).unwrap();
        let runtime = Arc::new(Mutex::new(Runtime::new()));
        let server_runtime = Arc::clone(&runtime);
        let server_thread = thread::spawn(move || {
            for _ in 0..2 {
                let (stream, _) = listener.accept().unwrap();
                handle_client(stream, &server_runtime).unwrap();
            }
        });

        let registration = register_provider(&socket_path).unwrap();
        let response = commit_promise(
            &socket_path,
            sample_commit_request(registration.provider_id),
        )
        .unwrap();

        assert_eq!(response.promise_id, "promise-1");
        server_thread.join().unwrap();
        assert!(runtime
            .lock()
            .unwrap()
            .promise("promise-1")
            .unwrap()
            .get("docs/readme.txt")
            .is_some());

        let _ = fs::remove_file(socket_path);
    }

    fn send_hello(stream: &mut UnixStream) {
        write_frame(
            stream,
            &Request::Hello {
                protocol_version: IPC_PROTOCOL_VERSION,
                api_version: API_VERSION,
            },
        )
        .unwrap();

        let response: Response = read_frame(stream).unwrap().unwrap();
        assert_eq!(
            response,
            Response::Hello {
                protocol_version: IPC_PROTOCOL_VERSION,
                api_version: API_VERSION,
            }
        );
    }

    fn unique_socket_path() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "fuse-promise-ipc-{}-{nanos}.sock",
            std::process::id()
        ))
    }

    fn sample_commit_request(provider_id: u64) -> PromiseCommitRequest {
        PromiseCommitRequest {
            provider_id,
            nodes: vec![
                PromiseNodeSpec {
                    kind: PromiseNodeKind::Directory,
                    relative_path: "docs".to_owned(),
                    provider_node_id: "remote-dir-1".to_owned(),
                    attr: PromiseNodeAttr {
                        mode: 0o755,
                        size: 0,
                        mtime_nsec: 0,
                    },
                },
                PromiseNodeSpec {
                    kind: PromiseNodeKind::File,
                    relative_path: "docs/readme.txt".to_owned(),
                    provider_node_id: "remote-file-1".to_owned(),
                    attr: PromiseNodeAttr {
                        mode: 0o644,
                        size: 12,
                        mtime_nsec: 0,
                    },
                },
            ],
        }
    }
}
