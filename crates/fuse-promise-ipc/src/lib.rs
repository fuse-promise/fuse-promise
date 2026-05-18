use bincode::{Decode, Encode};
use fuse_promise_runtime::{
    default_control_socket_path, default_mount_path, normalize_relative_path, NodeAttr,
    PromiseBuilder, Runtime, Status, API_VERSION,
};
use std::fmt::Write as _;
use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::fs::FileTypeExt;
use std::os::unix::net::UnixListener;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

pub const IPC_PROTOCOL_VERSION: u32 = 1;
pub const MAX_FRAME_LEN: u32 = 1024 * 1024;
pub const MAX_PROVIDER_READ_LEN: u32 = 256 * 1024;
const PROVIDER_READ_TIMEOUT: Duration = Duration::from_secs(30);

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
    ProviderReadResponse(ProviderReadResponseBody),
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
    ProviderReadRequest(ProviderReadRequestBody),
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

#[derive(Debug, Clone, Encode, Decode, PartialEq, Eq)]
struct ProviderReadRequestBody {
    request_id: u64,
    provider_id: u64,
    promise_id: String,
    relative_path: String,
    provider_node_id: String,
    offset: u64,
    length: u32,
}

#[derive(Debug, Clone, Encode, Decode, PartialEq, Eq)]
struct ProviderReadResponseBody {
    request_id: u64,
    status: ProviderReadStatusBody,
    bytes: Vec<u8>,
}

#[derive(Debug, Clone, Copy, Encode, Decode, PartialEq, Eq)]
enum ProviderReadStatusBody {
    Ok,
    InvalidArgument,
    Permission,
    NotFound,
    ProviderGone,
    Io,
    Timeout,
    Cancelled,
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

#[derive(Debug)]
pub struct ProviderConnection {
    stream: UnixStream,
    provider_id: u64,
}

impl ProviderConnection {
    pub fn provider_id(&self) -> u64 {
        self.provider_id
    }

    pub fn try_clone_stream(&self) -> io::Result<UnixStream> {
        self.stream.try_clone()
    }

    pub fn shutdown(&self) -> io::Result<()> {
        self.stream.shutdown(std::net::Shutdown::Both)
    }

    pub fn read_provider_read_request(&mut self) -> io::Result<Option<ProviderReadRequest>> {
        read_provider_read_request(&mut self.stream)
    }

    pub fn write_provider_read_response(
        &mut self,
        response: &ProviderReadResponse,
    ) -> io::Result<()> {
        write_provider_read_response(&mut self.stream, response)
    }

    pub fn unregister(mut self) -> io::Result<()> {
        write_frame(
            &mut self.stream,
            &Request::ProviderUnregister {
                provider_id: self.provider_id,
            },
        )?;
        match read_response(&mut self.stream)? {
            Response::ProviderUnregistered => Ok(()),
            Response::Error(error) => Err(error_to_io(error)),
            _ => Err(invalid_data(
                "daemon returned an unexpected provider unregister response",
            )),
        }
    }

    #[doc(hidden)]
    pub fn from_stream_for_test(stream: UnixStream, provider_id: u64) -> Self {
        Self {
            stream,
            provider_id,
        }
    }
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderReadRequest {
    pub request_id: u64,
    pub provider_id: u64,
    pub promise_id: String,
    pub relative_path: String,
    pub provider_node_id: String,
    pub offset: u64,
    pub length: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderReadResponse {
    pub request_id: u64,
    pub status: ProviderReadStatus,
    pub bytes: Vec<u8>,
}

#[derive(Clone)]
pub struct IpcState {
    runtime: Arc<Mutex<Runtime>>,
    provider_routes: Arc<Mutex<std::collections::BTreeMap<u64, ProviderRoute>>>,
    pending_reads: Arc<Mutex<std::collections::BTreeMap<u64, PendingRead>>>,
}

#[derive(Clone)]
struct ProviderRoute {
    writer: Arc<Mutex<UnixStream>>,
}

struct PendingRead {
    provider_id: u64,
    request: ProviderReadRequest,
    sender: mpsc::Sender<io::Result<ProviderReadResponse>>,
}

impl IpcState {
    pub fn new(runtime: Arc<Mutex<Runtime>>) -> Self {
        Self {
            runtime,
            provider_routes: Arc::new(Mutex::new(std::collections::BTreeMap::new())),
            pending_reads: Arc::new(Mutex::new(std::collections::BTreeMap::new())),
        }
    }

    pub fn runtime(&self) -> Arc<Mutex<Runtime>> {
        Arc::clone(&self.runtime)
    }

    pub fn route_provider_read(
        &self,
        request: ProviderReadRequest,
    ) -> io::Result<ProviderReadResponse> {
        let request_body = request.clone().into_body()?;
        let request_id = request_body.request_id;
        let provider_id = request_body.provider_id;
        let route = {
            let routes = self
                .provider_routes
                .lock()
                .map_err(|_| io::Error::other("provider route lock poisoned"))?;
            routes
                .get(&request.provider_id)
                .cloned()
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "provider not connected"))?
        };

        let (sender, receiver) = mpsc::channel();
        {
            let mut pending = self
                .pending_reads
                .lock()
                .map_err(|_| io::Error::other("provider pending lock poisoned"))?;
            if pending
                .insert(
                    request.request_id,
                    PendingRead {
                        provider_id: request.provider_id,
                        request,
                        sender,
                    },
                )
                .is_some()
            {
                return Err(invalid_data("duplicate provider read request id"));
            }
        }

        let write_result = {
            let mut writer = route
                .writer
                .lock()
                .map_err(|_| io::Error::other("provider writer lock poisoned"))?;
            write_frame(&mut *writer, &Response::ProviderReadRequest(request_body))
        };
        if let Err(error) = write_result {
            self.remove_pending_read(request_id);
            self.disconnect_provider_id(provider_id)?;
            return Err(error);
        }

        match receiver.recv_timeout(PROVIDER_READ_TIMEOUT) {
            Ok(response) => response,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                self.remove_pending_read(request_id);
                Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "provider read response timed out",
                ))
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "provider read response channel closed",
            )),
        }
    }

    fn register_provider_route(&self, provider_id: u64, stream: &UnixStream) -> io::Result<()> {
        let route = ProviderRoute {
            writer: Arc::new(Mutex::new(stream.try_clone()?)),
        };
        self.provider_routes
            .lock()
            .map_err(|_| io::Error::other("provider route lock poisoned"))?
            .insert(provider_id, route);
        Ok(())
    }

    fn complete_provider_read(
        &self,
        registered_providers: &[fuse_promise_runtime::ProviderId],
        response: ProviderReadResponseBody,
    ) -> io::Result<()> {
        let response = ProviderReadResponse::from_body(response)?;
        let pending = {
            let mut pending_reads = self
                .pending_reads
                .lock()
                .map_err(|_| io::Error::other("provider pending lock poisoned"))?;
            pending_reads.remove(&response.request_id).ok_or_else(|| {
                invalid_data("provider read response does not match a pending request")
            })?
        };
        let owns_provider = registered_providers
            .iter()
            .any(|provider_id| provider_id.raw() == pending.provider_id);
        let result = if owns_provider {
            validate_provider_read_response_for_request(&pending.request, &response)
                .map(|_| response)
        } else {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "provider read response came from the wrong provider connection",
            ))
        };

        let _ = pending.sender.send(result);
        Ok(())
    }

    fn disconnect_provider(&self, provider_id: fuse_promise_runtime::ProviderId) -> io::Result<()> {
        self.disconnect_provider_id(provider_id.raw())
    }

    fn disconnect_provider_id(&self, provider_id: u64) -> io::Result<()> {
        self.provider_routes
            .lock()
            .map_err(|_| io::Error::other("provider route lock poisoned"))?
            .remove(&provider_id);

        let Some(provider_id_value) = fuse_promise_runtime::ProviderId::from_raw(provider_id)
        else {
            return Ok(());
        };
        {
            let mut runtime = self
                .runtime
                .lock()
                .map_err(|_| io::Error::other("runtime lock poisoned"))?;
            let _ = runtime.unregister_provider(provider_id_value);
        }
        self.fail_provider_pending_reads(provider_id, io::ErrorKind::BrokenPipe);
        Ok(())
    }

    fn fail_provider_pending_reads(&self, provider_id: u64, error_kind: io::ErrorKind) {
        let pending_reads = {
            let mut pending = match self.pending_reads.lock() {
                Ok(pending) => pending,
                Err(_) => return,
            };
            let request_ids = pending
                .iter()
                .filter_map(|(request_id, pending)| {
                    (pending.provider_id == provider_id).then_some(*request_id)
                })
                .collect::<Vec<_>>();
            request_ids
                .into_iter()
                .filter_map(|request_id| pending.remove(&request_id))
                .collect::<Vec<_>>()
        };

        for pending in pending_reads {
            let _ = pending
                .sender
                .send(Err(io::Error::new(error_kind, "provider disconnected")));
        }
    }

    fn remove_pending_read(&self, request_id: u64) {
        if let Ok(mut pending) = self.pending_reads.lock() {
            pending.remove(&request_id);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderReadStatus {
    Ok,
    InvalidArgument,
    Permission,
    NotFound,
    ProviderGone,
    Io,
    Timeout,
    Cancelled,
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
    let connection = connect_provider(socket_path)?;
    Ok(ProviderRegistration {
        provider_id: connection.provider_id(),
    })
}

pub fn connect_provider(socket_path: &Path) -> io::Result<ProviderConnection> {
    let mut stream = connect_and_hello(socket_path)?;

    write_frame(&mut stream, &Request::ProviderRegister)?;
    match read_response(&mut stream)? {
        Response::ProviderRegistered { provider_id } => Ok(ProviderConnection {
            stream,
            provider_id,
        }),
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

pub fn write_provider_read_request<W>(
    writer: &mut W,
    request: &ProviderReadRequest,
) -> io::Result<()>
where
    W: Write,
{
    write_frame(
        writer,
        &Response::ProviderReadRequest(request.clone().into_body()?),
    )
}

pub fn read_provider_read_request<R>(reader: &mut R) -> io::Result<Option<ProviderReadRequest>>
where
    R: Read,
{
    match read_frame::<_, Response>(reader)? {
        Some(Response::ProviderReadRequest(request)) => {
            ProviderReadRequest::from_body(request).map(Some)
        }
        Some(Response::Error(error)) => Err(error_to_io(error)),
        Some(_) => Err(invalid_data(
            "non-provider-read response received where request was expected",
        )),
        None => Ok(None),
    }
}

pub fn write_provider_read_response<W>(
    writer: &mut W,
    response: &ProviderReadResponse,
) -> io::Result<()>
where
    W: Write,
{
    write_frame(
        writer,
        &Request::ProviderReadResponse(response.clone().into_body()?),
    )
}

pub fn read_provider_read_response<R>(reader: &mut R) -> io::Result<Option<ProviderReadResponse>>
where
    R: Read,
{
    match read_frame::<_, Request>(reader)? {
        Some(Request::ProviderReadResponse(response)) => {
            ProviderReadResponse::from_body(response).map(Some)
        }
        Some(_) => Err(invalid_data(
            "non-provider-read request received where response was expected",
        )),
        None => Ok(None),
    }
}

pub fn validate_provider_read_response_for_request(
    request: &ProviderReadRequest,
    response: &ProviderReadResponse,
) -> io::Result<()> {
    if response.request_id != request.request_id {
        return Err(invalid_data("provider read response id mismatch"));
    }
    if response.status == ProviderReadStatus::Ok && response.bytes.len() > request.length as usize {
        return Err(invalid_data(
            "provider read response exceeds requested length",
        ));
    }

    response.validate()
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
    serve_state(IpcState::new(runtime))
}

pub fn serve_state(state: IpcState) -> io::Result<()> {
    let socket_path = default_control_socket_path().map_err(status_to_io)?;
    bind_status_socket(&socket_path, state)
}

fn bind_status_socket(socket_path: &Path, state: IpcState) -> io::Result<()> {
    if let Some(parent) = socket_path.parent() {
        fs::create_dir_all(parent)?;
    }
    remove_stale_socket(socket_path)?;

    let listener = UnixListener::bind(socket_path)?;
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let state = state.clone();
                thread::spawn(move || {
                    let _ = handle_client_with_state(stream, &state);
                });
            }
            Err(error) => return Err(error),
        }
    }

    Ok(())
}

#[cfg(test)]
fn handle_client(stream: UnixStream, runtime: &Arc<Mutex<Runtime>>) -> io::Result<()> {
    let state = IpcState::new(Arc::clone(runtime));
    handle_client_with_state(stream, &state)
}

fn handle_client_with_state(mut stream: UnixStream, state: &IpcState) -> io::Result<()> {
    validate_peer(&stream)?;

    let mut registered_providers = Vec::new();
    let result = handle_client_requests(&mut stream, state, &mut registered_providers);
    let disconnect_result = disconnect_registered_providers(state, &registered_providers);

    result.and(disconnect_result)
}

fn handle_client_requests(
    stream: &mut UnixStream,
    state: &IpcState,
    registered_providers: &mut Vec<fuse_promise_runtime::ProviderId>,
) -> io::Result<()> {
    let mut negotiated = false;
    while let Some(request) = read_frame::<_, Request>(stream)? {
        match request {
            Request::Hello {
                protocol_version,
                api_version,
            } => {
                if protocol_version == IPC_PROTOCOL_VERSION && api_version == API_VERSION {
                    negotiated = true;
                    write_frame(
                        stream,
                        &Response::Hello {
                            protocol_version: IPC_PROTOCOL_VERSION,
                            api_version: API_VERSION,
                        },
                    )?;
                } else {
                    write_error(
                        stream,
                        ErrorCode::VersionMismatch,
                        "unsupported IPC protocol or API version",
                    )?;
                }
            }
            Request::Status if negotiated => {
                let runtime = state
                    .runtime
                    .lock()
                    .map_err(|_| io::Error::other("runtime lock poisoned"))?;
                let status = DaemonStatus::from_runtime(&runtime).map_err(status_to_io)?;
                write_frame(stream, &Response::Status(StatusBody::from_status(&status)))?;
            }
            Request::Status => {
                write_error(
                    stream,
                    ErrorCode::InvalidRequest,
                    "client must send hello before status",
                )?;
            }
            Request::ProviderRegister if negotiated => {
                let mut runtime = state
                    .runtime
                    .lock()
                    .map_err(|_| io::Error::other("runtime lock poisoned"))?;
                let provider_id = runtime.register_provider();
                drop(runtime);
                state.register_provider_route(provider_id.raw(), stream)?;
                registered_providers.push(provider_id);
                write_frame(
                    stream,
                    &Response::ProviderRegistered {
                        provider_id: provider_id.raw(),
                    },
                )?;
            }
            Request::ProviderRegister => {
                write_error(
                    stream,
                    ErrorCode::InvalidRequest,
                    "client must send hello before provider register",
                )?;
            }
            Request::ProviderUnregister { provider_id } if negotiated => {
                let Some(provider_id) = fuse_promise_runtime::ProviderId::from_raw(provider_id)
                else {
                    write_error(
                        stream,
                        ErrorCode::InvalidRequest,
                        "provider id must be nonzero",
                    )?;
                    continue;
                };

                let mut runtime = state
                    .runtime
                    .lock()
                    .map_err(|_| io::Error::other("runtime lock poisoned"))?;
                match runtime.unregister_provider(provider_id) {
                    Ok(()) => {
                        drop(runtime);
                        registered_providers.retain(|id| *id != provider_id);
                        state.disconnect_provider(provider_id)?;
                        write_frame(stream, &Response::ProviderUnregistered)?;
                    }
                    Err(status) => write_status_error(stream, status)?,
                }
            }
            Request::ProviderUnregister { .. } => {
                write_error(
                    stream,
                    ErrorCode::InvalidRequest,
                    "client must send hello before provider unregister",
                )?;
            }
            Request::PromiseCommit(request) if negotiated => {
                handle_promise_commit(stream, &state.runtime, request)?;
            }
            Request::PromiseCommit(_) => {
                write_error(
                    stream,
                    ErrorCode::InvalidRequest,
                    "client must send hello before promise commit",
                )?;
            }
            Request::ProviderReadResponse(response) if negotiated => {
                state.complete_provider_read(registered_providers, response)?;
            }
            Request::ProviderReadResponse(_) => {
                write_error(
                    stream,
                    ErrorCode::InvalidRequest,
                    "client must send hello before provider read response",
                )?;
            }
        }
    }

    Ok(())
}

fn disconnect_registered_providers(
    state: &IpcState,
    provider_ids: &[fuse_promise_runtime::ProviderId],
) -> io::Result<()> {
    if provider_ids.is_empty() {
        return Ok(());
    }

    for provider_id in provider_ids {
        state.disconnect_provider(*provider_id)?;
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

impl ProviderReadRequest {
    fn into_body(self) -> io::Result<ProviderReadRequestBody> {
        let normalized_path = validate_relative_path(&self.relative_path)?;
        validate_nonzero("provider read request id", self.request_id)?;
        validate_provider_id(self.provider_id)?;
        validate_token("promise id", &self.promise_id)?;
        validate_token("provider node id", &self.provider_node_id)?;
        validate_read_range(self.offset, self.length)?;

        Ok(ProviderReadRequestBody {
            request_id: self.request_id,
            provider_id: self.provider_id,
            promise_id: self.promise_id,
            relative_path: normalized_path,
            provider_node_id: self.provider_node_id,
            offset: self.offset,
            length: self.length,
        })
    }

    fn from_body(body: ProviderReadRequestBody) -> io::Result<Self> {
        ProviderReadRequest {
            request_id: body.request_id,
            provider_id: body.provider_id,
            promise_id: body.promise_id,
            relative_path: body.relative_path,
            provider_node_id: body.provider_node_id,
            offset: body.offset,
            length: body.length,
        }
        .validated()
    }

    fn validated(self) -> io::Result<Self> {
        let body = self.clone().into_body()?;
        Ok(Self {
            request_id: body.request_id,
            provider_id: body.provider_id,
            promise_id: body.promise_id,
            relative_path: body.relative_path,
            provider_node_id: body.provider_node_id,
            offset: body.offset,
            length: body.length,
        })
    }
}

impl ProviderReadResponse {
    fn into_body(self) -> io::Result<ProviderReadResponseBody> {
        self.validate()?;

        Ok(ProviderReadResponseBody {
            request_id: self.request_id,
            status: self.status.into_body(),
            bytes: self.bytes,
        })
    }

    fn from_body(body: ProviderReadResponseBody) -> io::Result<Self> {
        ProviderReadResponse {
            request_id: body.request_id,
            status: ProviderReadStatus::from_body(body.status),
            bytes: body.bytes,
        }
        .validated()
    }

    fn validated(self) -> io::Result<Self> {
        self.validate()?;
        Ok(self)
    }

    fn validate(&self) -> io::Result<()> {
        validate_nonzero("provider read response id", self.request_id)?;
        if self.bytes.len() > MAX_PROVIDER_READ_LEN as usize {
            return Err(invalid_data(
                "provider read response exceeds maximum read length",
            ));
        }
        if self.status != ProviderReadStatus::Ok && !self.bytes.is_empty() {
            return Err(invalid_data(
                "provider read error response must not include bytes",
            ));
        }

        Ok(())
    }
}

impl ProviderReadStatus {
    fn into_body(self) -> ProviderReadStatusBody {
        match self {
            ProviderReadStatus::Ok => ProviderReadStatusBody::Ok,
            ProviderReadStatus::InvalidArgument => ProviderReadStatusBody::InvalidArgument,
            ProviderReadStatus::Permission => ProviderReadStatusBody::Permission,
            ProviderReadStatus::NotFound => ProviderReadStatusBody::NotFound,
            ProviderReadStatus::ProviderGone => ProviderReadStatusBody::ProviderGone,
            ProviderReadStatus::Io => ProviderReadStatusBody::Io,
            ProviderReadStatus::Timeout => ProviderReadStatusBody::Timeout,
            ProviderReadStatus::Cancelled => ProviderReadStatusBody::Cancelled,
        }
    }

    fn from_body(body: ProviderReadStatusBody) -> Self {
        match body {
            ProviderReadStatusBody::Ok => ProviderReadStatus::Ok,
            ProviderReadStatusBody::InvalidArgument => ProviderReadStatus::InvalidArgument,
            ProviderReadStatusBody::Permission => ProviderReadStatus::Permission,
            ProviderReadStatusBody::NotFound => ProviderReadStatus::NotFound,
            ProviderReadStatusBody::ProviderGone => ProviderReadStatus::ProviderGone,
            ProviderReadStatusBody::Io => ProviderReadStatus::Io,
            ProviderReadStatusBody::Timeout => ProviderReadStatus::Timeout,
            ProviderReadStatusBody::Cancelled => ProviderReadStatus::Cancelled,
        }
    }
}

fn validate_nonzero(name: &str, value: u64) -> io::Result<()> {
    if value == 0 {
        Err(invalid_data(&format!("{name} must be nonzero")))
    } else {
        Ok(())
    }
}

fn validate_provider_id(provider_id: u64) -> io::Result<()> {
    if fuse_promise_runtime::ProviderId::from_raw(provider_id).is_some() {
        Ok(())
    } else {
        Err(invalid_data("provider id must be nonzero"))
    }
}

fn validate_token(name: &str, value: &str) -> io::Result<()> {
    if value.is_empty() {
        return Err(invalid_data(&format!("{name} must not be empty")));
    }
    if value.as_bytes().contains(&0) {
        return Err(invalid_data(&format!("{name} must not contain NUL")));
    }

    Ok(())
}

fn validate_relative_path(path: &str) -> io::Result<String> {
    normalize_relative_path(path).map_err(|status| invalid_data(status.as_str()))
}

fn validate_read_range(offset: u64, length: u32) -> io::Result<()> {
    if length == 0 {
        return Err(invalid_data("provider read length must be nonzero"));
    }
    if length > MAX_PROVIDER_READ_LEN {
        return Err(invalid_data("provider read length exceeds maximum"));
    }
    if offset.checked_add(u64::from(length)).is_none() {
        return Err(invalid_data("provider read range overflows"));
    }

    Ok(())
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
    use fuse_promise_runtime::{PromiseState, ProviderState};
    use std::io::Cursor;
    use std::os::unix::fs::PermissionsExt;
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
        let runtime_dir = tempfile::tempdir().unwrap();
        fs::set_permissions(runtime_dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        std::env::set_var("XDG_RUNTIME_DIR", runtime_dir.path());
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
    fn provider_connection_drop_marks_provider_disconnected() {
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

        drop(client);
        server_thread.join().unwrap().unwrap();

        let provider_id = fuse_promise_runtime::ProviderId::from_raw(provider_id).unwrap();
        assert_eq!(
            runtime.lock().unwrap().provider(provider_id).unwrap().state,
            ProviderState::Disconnected
        );
    }

    #[test]
    fn provider_connection_drop_marks_owned_promises_provider_gone() {
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

        assert_eq!(
            runtime.lock().unwrap().promise("promise-1").unwrap().state,
            PromiseState::ProviderGone
        );
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
    fn routes_provider_read_requests_to_registered_provider_connection() {
        let (provider_client, provider_server) = UnixStream::pair().unwrap();
        let runtime = Arc::new(Mutex::new(Runtime::new()));
        let state = IpcState::new(Arc::clone(&runtime));
        let server_state = state.clone();
        let server_thread =
            thread::spawn(move || handle_client_with_state(provider_server, &server_state));
        let mut provider = ProviderConnection::from_stream_for_test(provider_client, 1);

        send_hello(&mut provider.stream);
        write_frame(&mut provider.stream, &Request::ProviderRegister).unwrap();
        let provider_id = match read_frame(&mut provider.stream).unwrap().unwrap() {
            Response::ProviderRegistered { provider_id } => provider_id,
            other => panic!("unexpected response: {other:?}"),
        };

        let route_state = state.clone();
        let read_thread = thread::spawn(move || {
            route_state.route_provider_read(ProviderReadRequest {
                request_id: 99,
                provider_id,
                promise_id: "promise-1".to_owned(),
                relative_path: "docs/readme.txt".to_owned(),
                provider_node_id: "remote-file-1".to_owned(),
                offset: 3,
                length: 6,
            })
        });

        let request = provider.read_provider_read_request().unwrap().unwrap();
        assert_eq!(request.request_id, 99);
        assert_eq!(request.provider_id, provider_id);
        assert_eq!(request.offset, 3);
        provider
            .write_provider_read_response(&ProviderReadResponse {
                request_id: request.request_id,
                status: ProviderReadStatus::Ok,
                bytes: b"answer".to_vec(),
            })
            .unwrap();

        let response = read_thread.join().unwrap().unwrap();
        assert_eq!(response.status, ProviderReadStatus::Ok);
        assert_eq!(response.bytes, b"answer");

        provider.shutdown().unwrap();
        server_thread.join().unwrap().unwrap();
    }

    #[test]
    fn route_provider_read_rejects_wrong_provider_response_connection() {
        let (provider_one_client, provider_one_server) = UnixStream::pair().unwrap();
        let (provider_two_client, provider_two_server) = UnixStream::pair().unwrap();
        let runtime = Arc::new(Mutex::new(Runtime::new()));
        let state = IpcState::new(Arc::clone(&runtime));
        let provider_one_state = state.clone();
        let provider_two_state = state.clone();
        let provider_one_thread = thread::spawn(move || {
            handle_client_with_state(provider_one_server, &provider_one_state)
        });
        let provider_two_thread = thread::spawn(move || {
            handle_client_with_state(provider_two_server, &provider_two_state)
        });
        let mut provider_one = ProviderConnection::from_stream_for_test(provider_one_client, 1);
        let mut provider_two = ProviderConnection::from_stream_for_test(provider_two_client, 2);

        send_hello(&mut provider_one.stream);
        write_frame(&mut provider_one.stream, &Request::ProviderRegister).unwrap();
        let provider_one_id = match read_frame(&mut provider_one.stream).unwrap().unwrap() {
            Response::ProviderRegistered { provider_id } => provider_id,
            other => panic!("unexpected response: {other:?}"),
        };
        send_hello(&mut provider_two.stream);
        write_frame(&mut provider_two.stream, &Request::ProviderRegister).unwrap();
        let _provider_two_id = match read_frame(&mut provider_two.stream).unwrap().unwrap() {
            Response::ProviderRegistered { provider_id } => provider_id,
            other => panic!("unexpected response: {other:?}"),
        };

        let route_state = state.clone();
        let read_thread = thread::spawn(move || {
            route_state.route_provider_read(ProviderReadRequest {
                request_id: 12345,
                provider_id: provider_one_id,
                promise_id: "promise-1".to_owned(),
                relative_path: "docs/readme.txt".to_owned(),
                provider_node_id: "remote-file-1".to_owned(),
                offset: 0,
                length: 1,
            })
        });
        let request = provider_one.read_provider_read_request().unwrap().unwrap();
        assert_eq!(request.request_id, 12345);

        write_frame(
            &mut provider_two.stream,
            &Request::ProviderReadResponse(ProviderReadResponseBody {
                request_id: 12345,
                status: ProviderReadStatusBody::Ok,
                bytes: Vec::new(),
            }),
        )
        .unwrap();
        assert_eq!(
            read_thread.join().unwrap().unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );

        provider_one.shutdown().unwrap();
        provider_two.shutdown().unwrap();
        provider_one_thread.join().unwrap().unwrap();
        provider_two_thread.join().unwrap().unwrap();
    }

    #[test]
    fn commit_helper_uses_unix_socket() {
        let socket_path = unique_socket_path();
        let listener = UnixListener::bind(&socket_path).unwrap();
        let runtime = Arc::new(Mutex::new(Runtime::new()));
        let server_runtime = Arc::clone(&runtime);
        let server_thread = thread::spawn(move || {
            let mut children = Vec::new();
            for _ in 0..2 {
                let (stream, _) = listener.accept().unwrap();
                let runtime = Arc::clone(&server_runtime);
                children.push(thread::spawn(move || {
                    handle_client(stream, &runtime).unwrap();
                }));
            }
            for child in children {
                child.join().unwrap();
            }
        });

        let provider = connect_provider(&socket_path).unwrap();
        let response =
            commit_promise(&socket_path, sample_commit_request(provider.provider_id())).unwrap();

        assert_eq!(response.promise_id, "promise-1");
        drop(provider);
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

    #[test]
    fn provider_read_messages_round_trip() {
        let request = sample_read_request();
        let mut frame = Vec::new();
        write_provider_read_request(&mut frame, &request).unwrap();

        let decoded = read_provider_read_request(&mut Cursor::new(frame))
            .unwrap()
            .unwrap();
        assert_eq!(decoded.relative_path, "docs/readme.txt");
        assert_eq!(decoded.length, 12);

        let response = ProviderReadResponse {
            request_id: decoded.request_id,
            status: ProviderReadStatus::Ok,
            bytes: b"hello".to_vec(),
        };
        validate_provider_read_response_for_request(&decoded, &response).unwrap();

        let mut frame = Vec::new();
        write_provider_read_response(&mut frame, &response).unwrap();
        let decoded_response = read_provider_read_response(&mut Cursor::new(frame))
            .unwrap()
            .unwrap();

        assert_eq!(decoded_response, response);
    }

    #[test]
    fn provider_read_request_rejects_invalid_ranges() {
        let mut request = sample_read_request();
        request.length = 0;
        assert!(write_provider_read_request(&mut Vec::new(), &request).is_err());

        let mut request = sample_read_request();
        request.length = MAX_PROVIDER_READ_LEN + 1;
        assert!(write_provider_read_request(&mut Vec::new(), &request).is_err());

        let mut request = sample_read_request();
        request.offset = u64::MAX;
        assert!(write_provider_read_request(&mut Vec::new(), &request).is_err());
    }

    #[test]
    fn provider_read_request_rejects_invalid_identity_fields() {
        let mut request = sample_read_request();
        request.request_id = 0;
        assert!(write_provider_read_request(&mut Vec::new(), &request).is_err());

        let mut request = sample_read_request();
        request.provider_id = 0;
        assert!(write_provider_read_request(&mut Vec::new(), &request).is_err());

        let mut request = sample_read_request();
        request.promise_id.clear();
        assert!(write_provider_read_request(&mut Vec::new(), &request).is_err());

        let mut request = sample_read_request();
        request.relative_path = "../bad".to_owned();
        assert!(write_provider_read_request(&mut Vec::new(), &request).is_err());
    }

    #[test]
    fn provider_read_response_is_checked_against_request() {
        let request = sample_read_request();
        let mut response = ProviderReadResponse {
            request_id: request.request_id + 1,
            status: ProviderReadStatus::Ok,
            bytes: b"hello".to_vec(),
        };
        assert!(validate_provider_read_response_for_request(&request, &response).is_err());

        response.request_id = request.request_id;
        response.bytes = vec![1; request.length as usize + 1];
        assert!(validate_provider_read_response_for_request(&request, &response).is_err());

        response.status = ProviderReadStatus::ProviderGone;
        response.bytes = b"bad".to_vec();
        assert!(write_provider_read_response(&mut Vec::new(), &response).is_err());
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

    fn sample_read_request() -> ProviderReadRequest {
        ProviderReadRequest {
            request_id: 7,
            provider_id: 1,
            promise_id: "promise-1".to_owned(),
            relative_path: "docs//readme.txt".to_owned(),
            provider_node_id: "remote-file-1".to_owned(),
            offset: 0,
            length: 12,
        }
    }
}
