use bincode::{Decode, Encode};
use fuse_promise_runtime::{
    default_control_socket_path, default_mount_path, normalize_relative_path, CachePolicy,
    NodeAttr, NodeKind, PromiseBuilder, PromiseNode, PromiseState, Runtime, Status, API_VERSION,
};
use std::fmt::Write as _;
use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::fs::{
    DirBuilderExt, FileExt, FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt,
};
use std::os::unix::net::UnixListener;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
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
    pub cache_policy: &'static str,
    pub providers: usize,
    pub promises: usize,
}

impl DaemonStatus {
    pub fn from_runtime(runtime: &Runtime) -> Result<Self, Status> {
        Self::from_runtime_with_mount(runtime, IpcMountStatus::not_mounted())
    }

    pub fn from_runtime_with_mount(
        runtime: &Runtime,
        mount_status: IpcMountStatus,
    ) -> Result<Self, Status> {
        let mount_path = mount_status
            .mount_path
            .clone()
            .map(Ok)
            .unwrap_or_else(default_mount_path)?;
        Ok(Self {
            api_version: API_VERSION,
            mount_path,
            socket_path: default_control_socket_path()?,
            daemon: "connected",
            mount: mount_status.mount,
            fuse_adapter: mount_status.fuse_adapter,
            cache_policy: cache_policy_text(runtime.cache_policy()),
            providers: runtime.provider_count(),
            promises: runtime.promise_count(),
        })
    }

    pub fn encode(&self) -> String {
        StatusBody::from_status(self).encode_text()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IpcMountStatus {
    pub mount: &'static str,
    pub fuse_adapter: &'static str,
    mount_path: Option<PathBuf>,
    ready_for_commits: bool,
}

impl IpcMountStatus {
    pub fn not_mounted() -> Self {
        Self {
            mount: "not-mounted",
            fuse_adapter: "not-implemented",
            mount_path: None,
            ready_for_commits: false,
        }
    }

    pub fn disabled(mount_path: PathBuf) -> Self {
        Self {
            mount: "not-mounted",
            fuse_adapter: "disabled",
            mount_path: Some(mount_path),
            ready_for_commits: false,
        }
    }

    pub fn mounted(mount_path: PathBuf) -> Self {
        Self {
            mount: "mounted",
            fuse_adapter: "enabled",
            mount_path: Some(mount_path),
            ready_for_commits: false,
        }
    }

    pub fn commit_ready(mount_path: PathBuf) -> Self {
        Self {
            mount: "mounted",
            fuse_adapter: "enabled",
            mount_path: Some(mount_path),
            ready_for_commits: true,
        }
    }

    fn visible_promise_path(&self, promise_id: &str) -> Result<PathBuf, Status> {
        if !self.ready_for_commits {
            return Err(Status::Unavailable);
        }

        self.mount_path
            .as_ref()
            .map(|mount_path| mount_path.join(promise_id))
            .ok_or(Status::Unavailable)
    }

    fn resolve_visible_path(&self, source_path: &Path) -> Result<(String, String), Status> {
        if !self.ready_for_commits {
            return Err(Status::Unavailable);
        }
        if !source_path.is_absolute() {
            return Err(Status::InvalidArgument);
        }

        let mount_path = self.mount_path.as_ref().ok_or(Status::Unavailable)?;
        let relative = source_path
            .strip_prefix(mount_path)
            .map_err(|_| Status::NotFound)?;
        let mut components = relative.components();
        let Some(std::path::Component::Normal(promise_id)) = components.next() else {
            return Err(Status::NotFound);
        };
        let Some(promise_id) = promise_id.to_str() else {
            return Err(Status::InvalidArgument);
        };

        let mut parts = Vec::new();
        for component in components {
            let std::path::Component::Normal(part) = component else {
                return Err(Status::InvalidArgument);
            };
            let Some(part) = part.to_str() else {
                return Err(Status::InvalidArgument);
            };
            parts.push(part);
        }

        Ok((promise_id.to_owned(), parts.join("/")))
    }
}

#[derive(Debug, Clone, Encode, Decode, PartialEq, Eq)]
enum Request {
    Hello {
        protocol_version: u32,
        api_version: u32,
    },
    Status,
    Inspect,
    ProviderRegister,
    ProviderUnregister {
        provider_id: u64,
        provider_owner_token: u128,
    },
    PromiseCommit(PromiseCommitBody),
    Materialize(MaterializeBody),
    ProviderReadResponse(ProviderReadResponseBody),
}

#[derive(Debug, Clone, Encode, Decode, PartialEq, Eq)]
enum Response {
    Hello {
        protocol_version: u32,
        api_version: u32,
    },
    Status(StatusBody),
    Inspect(InspectBody),
    ProviderRegistered {
        provider_id: u64,
        provider_owner_token: u128,
    },
    ProviderUnregistered,
    PromiseCommitted(PromiseCommittedBody),
    Materialized(MaterializedBody),
    MaterializeFailed(MaterializeFailedBody),
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
    cache_policy: String,
    providers: u64,
    promises: u64,
}

#[derive(Debug, Clone, Encode, Decode, PartialEq, Eq)]
struct InspectBody {
    providers: u64,
    promises: Vec<InspectPromiseBody>,
}

#[derive(Debug, Clone, Encode, Decode, PartialEq, Eq)]
struct InspectPromiseBody {
    promise_id: String,
    provider_id: u64,
    state: String,
    nodes: Vec<InspectNodeBody>,
}

#[derive(Debug, Clone, Encode, Decode, PartialEq, Eq)]
struct InspectNodeBody {
    relative_path: String,
    inode: u64,
    kind: String,
    size: u64,
    mode: u32,
    provider_node_id: String,
}

#[derive(Debug, Clone, Encode, Decode, PartialEq, Eq)]
struct PromiseCommitBody {
    provider_id: u64,
    provider_owner_token: u128,
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
    visible_path: String,
}

#[derive(Debug, Clone, Encode, Decode, PartialEq, Eq)]
struct MaterializeBody {
    source_path: String,
    target_dir: String,
    conflict_policy: MaterializeConflictPolicyBody,
}

#[derive(Debug, Clone, Copy, Encode, Decode, PartialEq, Eq)]
enum MaterializeConflictPolicyBody {
    Fail,
    Overwrite,
    Rename,
}

#[derive(Debug, Clone, Encode, Decode, PartialEq, Eq)]
struct MaterializedBody {
    target_path: String,
    bytes_written: u64,
    files_written: u64,
    directories_created: u64,
}

#[derive(Debug, Clone, Encode, Decode, PartialEq, Eq)]
struct MaterializeFailedBody {
    code: ErrorCode,
    message: String,
    target_path: String,
    bytes_written: u64,
    files_written: u64,
    directories_created: u64,
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
            cache_policy: status.cache_policy.to_owned(),
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
        let _ = writeln!(output, "cache_policy={}", self.cache_policy);
        let _ = writeln!(output, "providers={}", self.providers);
        let _ = writeln!(output, "promises={}", self.promises);
        output
    }
}

impl InspectBody {
    fn from_runtime(runtime: &Runtime) -> Self {
        Self {
            providers: runtime.provider_count() as u64,
            promises: runtime
                .promises()
                .map(|tree| InspectPromiseBody {
                    promise_id: tree.promise_id.clone(),
                    provider_id: tree.provider_id.raw(),
                    state: promise_state_text(tree.state).to_owned(),
                    nodes: tree
                        .nodes
                        .values()
                        .map(|node| InspectNodeBody {
                            relative_path: if node.relative_path.is_empty() {
                                ".".to_owned()
                            } else {
                                node.relative_path.clone()
                            },
                            inode: node.inode,
                            kind: node_kind_text(node.kind).to_owned(),
                            size: node.attr.size,
                            mode: node.attr.mode,
                            provider_node_id: node.provider_node_id.clone(),
                        })
                        .collect(),
                })
                .collect(),
        }
    }

    fn encode_text(&self) -> String {
        let mut output = String::new();
        let _ = writeln!(output, "ok");
        let _ = writeln!(output, "providers={}", self.providers);
        let _ = writeln!(output, "promises={}", self.promises.len());
        for promise in &self.promises {
            let _ = writeln!(
                output,
                "promise id={} provider={} state={} nodes={}",
                promise.promise_id,
                promise.provider_id,
                promise.state,
                promise.nodes.len()
            );
            for node in &promise.nodes {
                let _ = writeln!(
                    output,
                    "node promise={} path={} inode={} kind={} size={} mode={:o} provider_node={}",
                    promise.promise_id,
                    node.relative_path,
                    node.inode,
                    node.kind,
                    node.size,
                    node.mode,
                    node.provider_node_id
                );
            }
        }
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
    Unavailable,
    NotFound,
    AlreadyExists,
    ProviderGone,
    Permission,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderRegistration {
    pub provider_id: u64,
    pub provider_owner_token: u128,
}

#[derive(Debug)]
pub struct ProviderConnection {
    stream: UnixStream,
    provider_id: u64,
    provider_owner_token: u128,
}

impl ProviderConnection {
    pub fn provider_id(&self) -> u64 {
        self.provider_id
    }

    pub fn provider_owner_token(&self) -> u128 {
        self.provider_owner_token
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
                provider_owner_token: self.provider_owner_token,
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
            provider_owner_token: 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromiseCommitRequest {
    pub provider_id: u64,
    pub provider_owner_token: u128,
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
    pub visible_path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaterializeConflictPolicy {
    Fail,
    Overwrite,
    Rename,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterializeRequest {
    pub source_path: PathBuf,
    pub target_dir: PathBuf,
    pub conflict_policy: MaterializeConflictPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterializeResponse {
    pub target_path: PathBuf,
    pub bytes_written: u64,
    pub files_written: u64,
    pub directories_created: u64,
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
    mount_status: Arc<Mutex<IpcMountStatus>>,
    provider_routes: Arc<Mutex<std::collections::BTreeMap<u64, ProviderRoute>>>,
    pending_reads: Arc<Mutex<std::collections::BTreeMap<u64, PendingRead>>>,
    next_read_request_id: Arc<AtomicU64>,
}

#[derive(Clone)]
struct ProviderRoute {
    writer: Arc<Mutex<UnixStream>>,
    owner_token: u128,
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
            mount_status: Arc::new(Mutex::new(IpcMountStatus::not_mounted())),
            provider_routes: Arc::new(Mutex::new(std::collections::BTreeMap::new())),
            pending_reads: Arc::new(Mutex::new(std::collections::BTreeMap::new())),
            next_read_request_id: Arc::new(AtomicU64::new(1)),
        }
    }

    pub fn runtime(&self) -> Arc<Mutex<Runtime>> {
        Arc::clone(&self.runtime)
    }

    pub fn set_mount_status(&self, mount_status: IpcMountStatus) -> io::Result<()> {
        *self
            .mount_status
            .lock()
            .map_err(|_| io::Error::other("mount status lock poisoned"))? = mount_status;
        Ok(())
    }

    fn mount_status(&self) -> io::Result<IpcMountStatus> {
        self.mount_status
            .lock()
            .map_err(|_| io::Error::other("mount status lock poisoned"))
            .map(|status| status.clone())
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

    pub fn next_provider_read_request_id(&self) -> u64 {
        self.next_read_request_id.fetch_add(1, Ordering::Relaxed)
    }

    fn register_provider_route(
        &self,
        provider_id: u64,
        owner_token: u128,
        stream: &UnixStream,
    ) -> io::Result<()> {
        let route = ProviderRoute {
            writer: Arc::new(Mutex::new(stream.try_clone()?)),
            owner_token,
        };
        self.provider_routes
            .lock()
            .map_err(|_| io::Error::other("provider route lock poisoned"))?
            .insert(provider_id, route);
        Ok(())
    }

    fn validate_provider_owner(
        &self,
        provider_id: fuse_promise_runtime::ProviderId,
        owner_token: u128,
    ) -> io::Result<std::result::Result<(), Status>> {
        let routes = self
            .provider_routes
            .lock()
            .map_err(|_| io::Error::other("provider route lock poisoned"))?;
        if let Some(route) = routes.get(&provider_id.raw()) {
            return if route.owner_token == owner_token {
                Ok(Ok(()))
            } else {
                Ok(Err(Status::Permission))
            };
        }
        drop(routes);

        let runtime = self
            .runtime
            .lock()
            .map_err(|_| io::Error::other("runtime lock poisoned"))?;
        if runtime.provider(provider_id).is_some() {
            Ok(Err(Status::ProviderGone))
        } else {
            Ok(Err(Status::NotFound))
        }
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

pub fn query_inspect(socket_path: &Path) -> io::Result<String> {
    let mut stream = connect_and_hello(socket_path)?;

    write_frame(&mut stream, &Request::Inspect)?;
    match read_response(&mut stream)? {
        Response::Inspect(inspect) => Ok(inspect.encode_text()),
        Response::Error(error) => Err(error_to_io(error)),
        _ => Err(invalid_data(
            "daemon returned an unexpected inspect response",
        )),
    }
}

pub fn register_provider(socket_path: &Path) -> io::Result<ProviderRegistration> {
    let connection = connect_provider(socket_path)?;
    Ok(ProviderRegistration {
        provider_id: connection.provider_id(),
        provider_owner_token: connection.provider_owner_token(),
    })
}

pub fn connect_provider(socket_path: &Path) -> io::Result<ProviderConnection> {
    let mut stream = connect_and_hello(socket_path)?;

    write_frame(&mut stream, &Request::ProviderRegister)?;
    match read_response(&mut stream)? {
        Response::ProviderRegistered {
            provider_id,
            provider_owner_token,
        } => Ok(ProviderConnection {
            stream,
            provider_id,
            provider_owner_token,
        }),
        Response::Error(error) => Err(error_to_io(error)),
        _ => Err(invalid_data(
            "daemon returned an unexpected provider register response",
        )),
    }
}

pub fn unregister_provider(
    socket_path: &Path,
    provider_id: u64,
    provider_owner_token: u128,
) -> io::Result<()> {
    let mut stream = connect_and_hello(socket_path)?;

    write_frame(
        &mut stream,
        &Request::ProviderUnregister {
            provider_id,
            provider_owner_token,
        },
    )?;
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
            visible_path: PathBuf::from(response.visible_path),
        }),
        Response::Error(error) => Err(error_to_io(error)),
        _ => Err(invalid_data(
            "daemon returned an unexpected promise commit response",
        )),
    }
}

pub fn materialize_file(
    socket_path: &Path,
    request: MaterializeRequest,
) -> io::Result<MaterializeResponse> {
    let mut stream = connect_and_hello(socket_path)?;

    write_frame(&mut stream, &Request::Materialize(request.into_body()?))?;
    match read_response(&mut stream)? {
        Response::Materialized(response) => Ok(MaterializeResponse {
            target_path: PathBuf::from(response.target_path),
            bytes_written: response.bytes_written,
            files_written: response.files_written,
            directories_created: response.directories_created,
        }),
        Response::MaterializeFailed(error) => Err(materialize_failure_to_io(error)),
        Response::Error(error) => Err(error_to_io(error)),
        _ => Err(invalid_data(
            "daemon returned an unexpected materialize response",
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
    validate_control_socket_for_connect(socket_path)?;
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
                let mount_status = state.mount_status()?;
                let runtime = state
                    .runtime
                    .lock()
                    .map_err(|_| io::Error::other("runtime lock poisoned"))?;
                let status = DaemonStatus::from_runtime_with_mount(&runtime, mount_status)
                    .map_err(status_to_io)?;
                write_frame(stream, &Response::Status(StatusBody::from_status(&status)))?;
            }
            Request::Status => {
                write_error(
                    stream,
                    ErrorCode::InvalidRequest,
                    "client must send hello before status",
                )?;
            }
            Request::Inspect if negotiated => {
                let runtime = state
                    .runtime
                    .lock()
                    .map_err(|_| io::Error::other("runtime lock poisoned"))?;
                write_frame(
                    stream,
                    &Response::Inspect(InspectBody::from_runtime(&runtime)),
                )?;
            }
            Request::Inspect => {
                write_error(
                    stream,
                    ErrorCode::InvalidRequest,
                    "client must send hello before inspect",
                )?;
            }
            Request::ProviderRegister if negotiated => {
                let mut runtime = state
                    .runtime
                    .lock()
                    .map_err(|_| io::Error::other("runtime lock poisoned"))?;
                let provider_id = runtime.register_provider();
                drop(runtime);
                let provider_owner_token = generate_provider_owner_token()?;
                state.register_provider_route(provider_id.raw(), provider_owner_token, stream)?;
                registered_providers.push(provider_id);
                write_frame(
                    stream,
                    &Response::ProviderRegistered {
                        provider_id: provider_id.raw(),
                        provider_owner_token,
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
            Request::ProviderUnregister {
                provider_id,
                provider_owner_token,
            } if negotiated => {
                let Some(provider_id) = fuse_promise_runtime::ProviderId::from_raw(provider_id)
                else {
                    write_error(
                        stream,
                        ErrorCode::InvalidRequest,
                        "provider id must be nonzero",
                    )?;
                    continue;
                };
                if let Err(status) =
                    state.validate_provider_owner(provider_id, provider_owner_token)?
                {
                    write_status_error(stream, status)?;
                    continue;
                }

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
                handle_promise_commit(stream, state, request)?;
            }
            Request::PromiseCommit(_) => {
                write_error(
                    stream,
                    ErrorCode::InvalidRequest,
                    "client must send hello before promise commit",
                )?;
            }
            Request::Materialize(request) if negotiated => {
                handle_materialize(stream, state, request)?;
            }
            Request::Materialize(_) => {
                write_error(
                    stream,
                    ErrorCode::InvalidRequest,
                    "client must send hello before materialize",
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
    state: &IpcState,
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
    if let Err(status) = state.validate_provider_owner(provider_id, request.provider_owner_token)? {
        write_status_error(stream, status)?;
        return Ok(());
    }

    let mount_status = state.mount_status()?;
    if !mount_status.ready_for_commits {
        write_status_error(stream, Status::Unavailable)?;
        return Ok(());
    }

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
        let mut runtime = state
            .runtime
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
    let visible_path = mount_status
        .visible_promise_path(&tree.promise_id)
        .map_err(status_to_io)?;

    write_frame(
        stream,
        &Response::PromiseCommitted(PromiseCommittedBody {
            promise_id: tree.promise_id,
            visible_path: visible_path.to_string_lossy().into_owned(),
        }),
    )
}

fn handle_materialize(
    stream: &mut UnixStream,
    state: &IpcState,
    request: MaterializeBody,
) -> io::Result<()> {
    match materialize_node_in_state(state, request) {
        Ok(response) => write_frame(stream, &Response::Materialized(response)),
        Err(error) => match error.progress {
            Some(ref progress) if progress.has_partial() => write_frame(
                stream,
                &Response::MaterializeFailed(error.to_failed_body(progress)),
            ),
            _ => write_status_error(stream, error.status),
        },
    }
}

struct MaterializePlan {
    promise_id: String,
    target_path: PathBuf,
    entries: Vec<MaterializeEntryPlan>,
}

#[derive(Debug, Clone)]
struct MaterializeEntryPlan {
    relative_path: String,
    target_path: PathBuf,
    kind: NodeKind,
    size: u64,
    mode: u32,
    mtime_nsec: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileIdentity {
    dev: u64,
    ino: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MaterializeTargetKind {
    File,
    Directory,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CreatedTarget {
    path: PathBuf,
    identity: FileIdentity,
    kind: MaterializeTargetKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MaterializeProgress {
    target_path: PathBuf,
    bytes_written: u64,
    files_written: u64,
    directories_created: u64,
}

#[derive(Debug)]
struct MaterializeOperationError {
    status: Status,
    progress: Option<MaterializeProgress>,
}

#[derive(Debug)]
struct MaterializeFileOutcome {
    bytes_written: u64,
    target_identity: FileIdentity,
}

#[derive(Debug)]
struct MaterializeWriteError {
    status: Status,
    created_target: Option<FileIdentity>,
    bytes_written: u64,
}

impl FileIdentity {
    fn from_metadata(metadata: &fs::Metadata) -> Self {
        Self {
            dev: metadata.dev(),
            ino: metadata.ino(),
        }
    }
}

impl MaterializeWriteError {
    fn without_created_target(status: Status) -> Self {
        Self {
            status,
            created_target: None,
            bytes_written: 0,
        }
    }

    fn with_created_target(
        status: Status,
        created_target: FileIdentity,
        bytes_written: u64,
    ) -> Self {
        Self {
            status,
            created_target: Some(created_target),
            bytes_written,
        }
    }
}

impl MaterializeProgress {
    fn new(target_path: PathBuf) -> Self {
        Self {
            target_path,
            bytes_written: 0,
            files_written: 0,
            directories_created: 0,
        }
    }

    fn has_partial(&self) -> bool {
        self.bytes_written > 0 || self.files_written > 0 || self.directories_created > 0
    }
}

impl MaterializeOperationError {
    fn without_progress(status: Status) -> Self {
        Self {
            status,
            progress: None,
        }
    }

    fn with_progress(status: Status, progress: MaterializeProgress) -> Self {
        Self {
            status,
            progress: Some(progress),
        }
    }

    fn to_failed_body(&self, progress: &MaterializeProgress) -> MaterializeFailedBody {
        MaterializeFailedBody {
            code: status_to_error_code(self.status),
            message: self.status.as_str().to_owned(),
            target_path: progress.target_path.to_string_lossy().into_owned(),
            bytes_written: progress.bytes_written,
            files_written: progress.files_written,
            directories_created: progress.directories_created,
        }
    }
}

fn materialize_node_in_state(
    state: &IpcState,
    request: MaterializeBody,
) -> std::result::Result<MaterializedBody, MaterializeOperationError> {
    let request = MaterializeRequest::from_body(request)
        .map_err(MaterializeOperationError::without_progress)?;
    let plan =
        plan_materialize(state, &request).map_err(MaterializeOperationError::without_progress)?;
    let mut progress = MaterializeProgress::new(plan.target_path.clone());
    let mut created_targets = Vec::new();

    for entry in plan
        .entries
        .iter()
        .filter(|entry| entry.kind == NodeKind::Directory)
    {
        match create_materialized_directory(entry) {
            Ok(created) => {
                progress.directories_created += 1;
                created_targets.push(created);
            }
            Err(status) => {
                cleanup_created_targets(&created_targets);
                return Err(MaterializeOperationError::with_progress(status, progress));
            }
        }
    }

    for entry in plan
        .entries
        .iter()
        .filter(|entry| entry.kind == NodeKind::File)
    {
        match write_materialized_file(state, &plan.promise_id, entry) {
            Ok(outcome) => {
                progress.bytes_written += outcome.bytes_written;
                progress.files_written += 1;
                created_targets.push(CreatedTarget {
                    path: entry.target_path.clone(),
                    identity: outcome.target_identity,
                    kind: MaterializeTargetKind::File,
                });
            }
            Err(error) => {
                progress.bytes_written += error.bytes_written;
                if let Some(identity) = error.created_target {
                    created_targets.push(CreatedTarget {
                        path: entry.target_path.clone(),
                        identity,
                        kind: MaterializeTargetKind::File,
                    });
                }
                cleanup_created_targets(&created_targets);
                return Err(MaterializeOperationError::with_progress(
                    error.status,
                    progress,
                ));
            }
        }
    }

    for entry in plan
        .entries
        .iter()
        .rev()
        .filter(|entry| entry.kind == NodeKind::Directory)
    {
        let Some(created) = created_targets.iter().find(|created| {
            created.kind == MaterializeTargetKind::Directory && created.path == entry.target_path
        }) else {
            cleanup_created_targets(&created_targets);
            return Err(MaterializeOperationError::with_progress(
                Status::Io,
                progress,
            ));
        };
        if let Err(status) = apply_directory_metadata(entry, created.identity) {
            cleanup_created_targets(&created_targets);
            return Err(MaterializeOperationError::with_progress(status, progress));
        }
    }

    {
        let mut runtime = state.runtime.lock().map_err(|_| {
            cleanup_created_targets(&created_targets);
            MaterializeOperationError::with_progress(Status::Io, progress.clone())
        })?;
        for entry in &plan.entries {
            if let Err(status) = runtime.mark_node_materialized(
                &plan.promise_id,
                &entry.relative_path,
                &entry.target_path,
            ) {
                cleanup_created_targets(&created_targets);
                return Err(MaterializeOperationError::with_progress(status, progress));
            }
        }
    }

    Ok(MaterializedBody {
        target_path: plan.target_path.to_string_lossy().into_owned(),
        bytes_written: progress.bytes_written,
        files_written: progress.files_written,
        directories_created: progress.directories_created,
    })
}

fn plan_materialize(
    state: &IpcState,
    request: &MaterializeRequest,
) -> std::result::Result<MaterializePlan, Status> {
    if request.conflict_policy != MaterializeConflictPolicy::Fail {
        return Err(Status::Unavailable);
    }
    validate_target_dir(&request.target_dir)?;

    let mount_status = state.mount_status().map_err(|_| Status::Io)?;
    let (promise_id, relative_path) = mount_status.resolve_visible_path(&request.source_path)?;
    let nodes = {
        let runtime = state.runtime.lock().map_err(|_| Status::Io)?;
        let tree = runtime.promise(&promise_id).ok_or(Status::NotFound)?;
        if tree.state != PromiseState::Available || !runtime.has_provider(tree.provider_id) {
            return Err(Status::ProviderGone);
        }
        tree.subtree_nodes(&relative_path)?
    };
    let root = nodes.first().ok_or(Status::NotFound)?;
    let target_path = request
        .target_dir
        .join(materialize_target_name(&promise_id, root)?);
    let mut entries = Vec::new();
    for node in nodes {
        let suffix = subtree_target_suffix(&relative_path, &node.relative_path)?;
        let entry_target = if suffix.is_empty() {
            target_path.clone()
        } else {
            target_path.join(suffix)
        };
        entries.push(MaterializeEntryPlan {
            relative_path: node.relative_path,
            target_path: entry_target,
            kind: node.kind,
            size: node.attr.size,
            mode: node.attr.mode,
            mtime_nsec: node.attr.mtime_nsec,
        });
    }
    preflight_materialize_targets(&entries)?;

    Ok(MaterializePlan {
        promise_id,
        target_path,
        entries,
    })
}

fn materialize_target_name(
    promise_id: &str,
    node: &PromiseNode,
) -> std::result::Result<String, Status> {
    if node.relative_path.is_empty() {
        Ok(promise_id.to_owned())
    } else if node.name.is_empty() {
        Err(Status::InvalidArgument)
    } else {
        Ok(node.name.clone())
    }
}

fn subtree_target_suffix(
    source_relative_path: &str,
    node_relative_path: &str,
) -> std::result::Result<String, Status> {
    if node_relative_path == source_relative_path {
        return Ok(String::new());
    }
    if source_relative_path.is_empty() {
        return Ok(node_relative_path.to_owned());
    }

    node_relative_path
        .strip_prefix(&format!("{source_relative_path}/"))
        .map(str::to_owned)
        .ok_or(Status::InvalidArgument)
}

fn preflight_materialize_targets(
    entries: &[MaterializeEntryPlan],
) -> std::result::Result<(), Status> {
    for entry in entries {
        match fs::symlink_metadata(&entry.target_path) {
            Ok(_) => return Err(Status::AlreadyExists),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(io_error_to_status(&error)),
        }
    }

    Ok(())
}

fn validate_target_dir(target_dir: &Path) -> std::result::Result<(), Status> {
    if !target_dir.is_absolute() {
        return Err(Status::InvalidArgument);
    }
    let metadata = fs::symlink_metadata(target_dir).map_err(|error| io_error_to_status(&error))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        Err(Status::InvalidArgument)
    } else {
        Ok(())
    }
}

fn create_materialized_directory(
    entry: &MaterializeEntryPlan,
) -> std::result::Result<CreatedTarget, Status> {
    let mut builder = fs::DirBuilder::new();
    builder.mode(0o700);
    builder
        .create(&entry.target_path)
        .map_err(|error| io_error_to_status(&error))?;
    let identity = FileIdentity::from_metadata(
        &fs::symlink_metadata(&entry.target_path).map_err(|error| io_error_to_status(&error))?,
    );

    Ok(CreatedTarget {
        path: entry.target_path.clone(),
        identity,
        kind: MaterializeTargetKind::Directory,
    })
}

fn apply_directory_metadata(
    entry: &MaterializeEntryPlan,
    expected: FileIdentity,
) -> std::result::Result<(), Status> {
    let directory =
        fs::File::open(&entry.target_path).map_err(|error| io_error_to_status(&error))?;
    let metadata = directory
        .metadata()
        .map_err(|error| io_error_to_status(&error))?;
    if FileIdentity::from_metadata(&metadata) != expected {
        return Err(Status::Unavailable);
    }
    directory
        .set_permissions(fs::Permissions::from_mode(entry.mode & 0o7777))
        .map_err(|error| io_error_to_status(&error))?;
    apply_mtime(&directory, entry.mtime_nsec)
}

fn cleanup_created_targets(created_targets: &[CreatedTarget]) {
    for target in created_targets
        .iter()
        .filter(|target| target.kind == MaterializeTargetKind::Directory)
    {
        restore_created_directory_permissions(target);
    }

    for target in created_targets.iter().rev() {
        let Ok(metadata) = fs::symlink_metadata(&target.path) else {
            continue;
        };
        if FileIdentity::from_metadata(&metadata) != target.identity {
            continue;
        }
        match target.kind {
            MaterializeTargetKind::File => {
                let _ = fs::remove_file(&target.path);
            }
            MaterializeTargetKind::Directory => {
                let _ = fs::remove_dir(&target.path);
            }
        }
    }
}

fn restore_created_directory_permissions(target: &CreatedTarget) {
    let Ok(metadata) = fs::symlink_metadata(&target.path) else {
        return;
    };
    if FileIdentity::from_metadata(&metadata) != target.identity {
        return;
    }
    let _ = fs::set_permissions(&target.path, fs::Permissions::from_mode(0o700));
}

fn write_materialized_file(
    state: &IpcState,
    promise_id: &str,
    entry: &MaterializeEntryPlan,
) -> std::result::Result<MaterializeFileOutcome, MaterializeWriteError> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&entry.target_path)
        .map_err(|error| {
            MaterializeWriteError::without_created_target(io_error_to_status(&error))
        })?;
    let target_identity = FileIdentity::from_metadata(&file.metadata().map_err(|error| {
        MaterializeWriteError::without_created_target(io_error_to_status(&error))
    })?);

    let mut offset = 0;
    while offset < entry.size {
        let length = (entry.size - offset).min(u64::from(MAX_PROVIDER_READ_LEN)) as u32;
        let bytes =
            read_materialize_chunk(state, promise_id, entry, offset, length).map_err(|status| {
                MaterializeWriteError::with_created_target(status, target_identity, offset)
            })?;
        if bytes.is_empty() {
            return Err(MaterializeWriteError::with_created_target(
                Status::Io,
                target_identity,
                offset,
            ));
        }

        file.write_all(&bytes).map_err(|error| {
            MaterializeWriteError::with_created_target(
                io_error_to_status(&error),
                target_identity,
                offset,
            )
        })?;
        offset = offset.checked_add(bytes.len() as u64).ok_or_else(|| {
            MaterializeWriteError::with_created_target(
                Status::InvalidArgument,
                target_identity,
                offset,
            )
        })?;
    }

    file.set_permissions(fs::Permissions::from_mode(entry.mode & 0o7777))
        .map_err(|error| {
            MaterializeWriteError::with_created_target(
                io_error_to_status(&error),
                target_identity,
                offset,
            )
        })?;
    apply_mtime(&file, entry.mtime_nsec).map_err(|status| {
        MaterializeWriteError::with_created_target(status, target_identity, offset)
    })?;
    drop(file);
    Ok(MaterializeFileOutcome {
        bytes_written: offset,
        target_identity,
    })
}

fn read_materialize_chunk(
    state: &IpcState,
    promise_id: &str,
    entry: &MaterializeEntryPlan,
    offset: u64,
    length: u32,
) -> std::result::Result<Vec<u8>, Status> {
    let read_plan = {
        let runtime = state.runtime.lock().map_err(|_| Status::Io)?;
        runtime.plan_read(promise_id, &entry.relative_path, offset, length)?
    };
    match read_plan {
        fuse_promise_runtime::ReadPlan::Request(plan) => {
            let request = ProviderReadRequest {
                request_id: state.next_provider_read_request_id(),
                provider_id: plan.provider_id.raw(),
                promise_id: plan.promise_id,
                relative_path: plan.relative_path,
                provider_node_id: plan.provider_node_id,
                offset: plan.offset,
                length: plan.length,
            };
            let response = state
                .route_provider_read(request)
                .map_err(|error| io_error_to_status(&error))?;
            if response.status == ProviderReadStatus::Ok {
                Ok(response.bytes)
            } else {
                Err(provider_read_status_to_status(response.status))
            }
        }
        fuse_promise_runtime::ReadPlan::Materialized(plan) => {
            read_local_materialized_chunk(&plan.path, plan.offset, plan.length)
        }
        fuse_promise_runtime::ReadPlan::Cached(plan) => Ok(plan.bytes),
        fuse_promise_runtime::ReadPlan::Eof => Err(Status::Io),
    }
}

fn read_local_materialized_chunk(
    path: &Path,
    offset: u64,
    length: u32,
) -> std::result::Result<Vec<u8>, Status> {
    let file = fs::File::open(path).map_err(|error| io_error_to_status(&error))?;
    let mut bytes = vec![0_u8; length as usize];
    let read = file
        .read_at(&mut bytes, offset)
        .map_err(|error| io_error_to_status(&error))?;
    if read != bytes.len() {
        return Err(Status::Io);
    }
    bytes.truncate(read);
    Ok(bytes)
}

fn apply_mtime(file: &fs::File, mtime_nsec: i64) -> std::result::Result<(), Status> {
    let seconds = mtime_nsec / 1_000_000_000;
    let nanoseconds = mtime_nsec % 1_000_000_000;
    let timestamp = rustix::time::Timespec {
        tv_sec: seconds,
        tv_nsec: nanoseconds as _,
    };
    let timestamps = rustix::fs::Timestamps {
        last_access: timestamp,
        last_modification: timestamp,
    };
    rustix::fs::futimens(file, &timestamps).map_err(|error| {
        let io_error: io::Error = error.into();
        io_error_to_status(&io_error)
    })
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
    if peer.uid.as_raw() != current_uid() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "IPC peer uid does not match current user",
        ));
    }

    Ok(())
}

fn validate_control_socket_for_connect(socket_path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(socket_path)?;
    if !metadata.file_type().is_socket() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "control socket path is not a socket",
        ));
    }
    if metadata.uid() != current_uid() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "control socket is not owned by the current user",
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
    if metadata.uid() != current_uid() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "stale control socket is not owned by the current user",
        ));
    }

    match fs::remove_file(socket_path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn current_uid() -> u32 {
    rustix::process::getuid().as_raw()
}

fn generate_provider_owner_token() -> io::Result<u128> {
    let mut random = fs::File::open("/dev/urandom")?;
    let mut bytes = [0u8; 16];
    loop {
        random.read_exact(&mut bytes)?;
        let token = u128::from_ne_bytes(bytes);
        if token != 0 {
            return Ok(token);
        }
    }
}

fn error_to_io(error: ErrorBody) -> io::Error {
    io::Error::new(error_code_to_io_kind(error.code), error.message)
}

fn materialize_failure_to_io(error: MaterializeFailedBody) -> io::Error {
    io::Error::new(
        error_code_to_io_kind(error.code),
        format!(
            "{}; target_path={}; bytes_written={}; files_written={}; directories_created={}",
            error.message,
            error.target_path,
            error.bytes_written,
            error.files_written,
            error.directories_created
        ),
    )
}

fn error_code_to_io_kind(code: ErrorCode) -> io::ErrorKind {
    match code {
        ErrorCode::InvalidRequest | ErrorCode::VersionMismatch => io::ErrorKind::InvalidData,
        ErrorCode::Unavailable | ErrorCode::NotFound => io::ErrorKind::NotFound,
        ErrorCode::ProviderGone => io::ErrorKind::BrokenPipe,
        ErrorCode::AlreadyExists => io::ErrorKind::AlreadyExists,
        ErrorCode::Permission => io::ErrorKind::PermissionDenied,
        ErrorCode::Internal => io::ErrorKind::Other,
    }
}

fn status_to_error_code(status: Status) -> ErrorCode {
    match status {
        Status::InvalidArgument => ErrorCode::InvalidRequest,
        Status::Unavailable => ErrorCode::Unavailable,
        Status::Permission => ErrorCode::Permission,
        Status::NotFound => ErrorCode::NotFound,
        Status::AlreadyExists => ErrorCode::AlreadyExists,
        Status::ProviderGone => ErrorCode::ProviderGone,
        Status::VersionMismatch => ErrorCode::VersionMismatch,
        _ => ErrorCode::Internal,
    }
}

fn io_error_to_status(error: &io::Error) -> Status {
    match error.kind() {
        io::ErrorKind::InvalidInput | io::ErrorKind::InvalidData => Status::InvalidArgument,
        io::ErrorKind::NotFound => Status::NotFound,
        io::ErrorKind::AlreadyExists => Status::AlreadyExists,
        io::ErrorKind::PermissionDenied => Status::Permission,
        io::ErrorKind::TimedOut => Status::Timeout,
        io::ErrorKind::ConnectionRefused
        | io::ErrorKind::ConnectionReset
        | io::ErrorKind::BrokenPipe => Status::ProviderGone,
        _ => Status::Io,
    }
}

fn provider_read_status_to_status(status: ProviderReadStatus) -> Status {
    match status {
        ProviderReadStatus::Ok => Status::Ok,
        ProviderReadStatus::InvalidArgument => Status::InvalidArgument,
        ProviderReadStatus::Permission => Status::Permission,
        ProviderReadStatus::NotFound => Status::NotFound,
        ProviderReadStatus::ProviderGone => Status::ProviderGone,
        ProviderReadStatus::Io => Status::Io,
        ProviderReadStatus::Timeout => Status::Timeout,
        ProviderReadStatus::Cancelled => Status::Cancelled,
    }
}

impl PromiseCommitRequest {
    pub fn from_builder_with_owner_token(
        builder: &PromiseBuilder,
        provider_owner_token: u128,
    ) -> Self {
        Self {
            provider_id: builder.provider_id().raw(),
            provider_owner_token,
            nodes: builder
                .nodes()
                .filter(|node| !node.relative_path.is_empty())
                .map(|node| PromiseNodeSpec {
                    kind: match node.kind {
                        NodeKind::File => PromiseNodeKind::File,
                        NodeKind::Directory => PromiseNodeKind::Directory,
                    },
                    relative_path: node.relative_path.clone(),
                    provider_node_id: node.provider_node_id.clone(),
                    attr: PromiseNodeAttr {
                        mode: node.attr.mode,
                        size: node.attr.size,
                        mtime_nsec: node.attr.mtime_nsec,
                    },
                })
                .collect(),
        }
    }

    fn into_body(self) -> PromiseCommitBody {
        PromiseCommitBody {
            provider_id: self.provider_id,
            provider_owner_token: self.provider_owner_token,
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

impl MaterializeResponse {
    pub fn encode_text(&self) -> String {
        let mut output = String::new();
        let _ = writeln!(output, "ok");
        let _ = writeln!(output, "target_path={}", self.target_path.display());
        let _ = writeln!(output, "bytes_written={}", self.bytes_written);
        let _ = writeln!(output, "files_written={}", self.files_written);
        let _ = writeln!(output, "directories_created={}", self.directories_created);
        output
    }
}

impl MaterializeRequest {
    fn into_body(self) -> io::Result<MaterializeBody> {
        validate_materialize_path("source path", &self.source_path)?;
        validate_materialize_path("target directory", &self.target_dir)?;
        Ok(MaterializeBody {
            source_path: self.source_path.to_string_lossy().into_owned(),
            target_dir: self.target_dir.to_string_lossy().into_owned(),
            conflict_policy: self.conflict_policy.into_body(),
        })
    }

    fn from_body(body: MaterializeBody) -> std::result::Result<Self, Status> {
        if body.source_path.is_empty() || body.target_dir.is_empty() {
            return Err(Status::InvalidArgument);
        }
        Ok(Self {
            source_path: PathBuf::from(body.source_path),
            target_dir: PathBuf::from(body.target_dir),
            conflict_policy: MaterializeConflictPolicy::from_body(body.conflict_policy),
        })
    }
}

impl MaterializeConflictPolicy {
    fn into_body(self) -> MaterializeConflictPolicyBody {
        match self {
            MaterializeConflictPolicy::Fail => MaterializeConflictPolicyBody::Fail,
            MaterializeConflictPolicy::Overwrite => MaterializeConflictPolicyBody::Overwrite,
            MaterializeConflictPolicy::Rename => MaterializeConflictPolicyBody::Rename,
        }
    }

    fn from_body(body: MaterializeConflictPolicyBody) -> Self {
        match body {
            MaterializeConflictPolicyBody::Fail => MaterializeConflictPolicy::Fail,
            MaterializeConflictPolicyBody::Overwrite => MaterializeConflictPolicy::Overwrite,
            MaterializeConflictPolicyBody::Rename => MaterializeConflictPolicy::Rename,
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

fn validate_materialize_path(name: &str, path: &Path) -> io::Result<()> {
    if path.as_os_str().is_empty() {
        return Err(invalid_data(&format!("{name} must not be empty")));
    }
    if path.to_str().is_none() {
        return Err(invalid_data(&format!("{name} must be UTF-8")));
    }

    Ok(())
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

fn promise_state_text(state: PromiseState) -> &'static str {
    match state {
        PromiseState::Available => "available",
        PromiseState::ProviderGone => "provider-gone",
        PromiseState::Materialized => "materialized",
    }
}

fn cache_policy_text(policy: CachePolicy) -> &'static str {
    policy.as_str()
}

fn node_kind_text(kind: NodeKind) -> &'static str {
    match kind {
        NodeKind::File => "file",
        NodeKind::Directory => "directory",
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
    use fuse_promise_runtime::{PromiseState, ProviderState};
    use std::io::Cursor;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::{Mutex as TestMutex, MutexGuard as TestMutexGuard, OnceLock};
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
            cache_policy: "no-cache",
            providers: 2,
            promises: 3,
        };

        let encoded = status.encode();
        assert!(encoded.starts_with("ok\n"));
        assert!(encoded.contains("api_version=1\n"));
        assert!(encoded.contains("cache_policy=no-cache\n"));
        assert!(encoded.contains("providers=2\n"));
        assert!(encoded.contains("promises=3\n"));
    }

    #[test]
    fn status_reports_read_through_cache_policy() {
        let _env_lock = xdg_runtime_env_lock();
        let runtime_dir = tempfile::tempdir().unwrap();
        fs::set_permissions(runtime_dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        std::env::set_var("XDG_RUNTIME_DIR", runtime_dir.path());
        let runtime = Runtime::with_cache_policy(CachePolicy::read_through(4096).unwrap()).unwrap();
        let status = DaemonStatus::from_runtime(&runtime).unwrap();

        assert_eq!(status.cache_policy, "read-through");
        assert!(status.encode().contains("cache_policy=read-through\n"));
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
            cache_policy: "no-cache".to_owned(),
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
        let _env_lock = xdg_runtime_env_lock();
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
                assert_eq!(status.cache_policy, "no-cache");
            }
            other => panic!("unexpected response: {other:?}"),
        }

        drop(client);
        server_thread.join().unwrap().unwrap();
    }

    #[test]
    fn status_uses_shared_mount_state() {
        let _env_lock = xdg_runtime_env_lock();
        let runtime_dir = tempfile::tempdir().unwrap();
        fs::set_permissions(runtime_dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        std::env::set_var("XDG_RUNTIME_DIR", runtime_dir.path());
        let (mut client, server) = UnixStream::pair().unwrap();
        let state = IpcState::new(Arc::new(Mutex::new(Runtime::new())));
        state
            .set_mount_status(IpcMountStatus::mounted(PathBuf::from("/tmp/fuse-promise")))
            .unwrap();
        let server_state = state.clone();
        let server_thread = thread::spawn(move || handle_client_with_state(server, &server_state));

        send_hello(&mut client);
        write_frame(&mut client, &Request::Status).unwrap();
        let response: Response = read_frame(&mut client).unwrap().unwrap();

        match response {
            Response::Status(status) => {
                assert_eq!(status.mount, "mounted");
                assert_eq!(status.fuse_adapter, "enabled");
                assert_eq!(status.cache_policy, "no-cache");
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
        let (provider_id, provider_owner_token) = registered_provider(response);
        assert_eq!(provider_id, 1);

        write_frame(
            &mut client,
            &Request::ProviderUnregister {
                provider_id,
                provider_owner_token,
            },
        )
        .unwrap();
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
            &Request::ProviderUnregister {
                provider_id: 99,
                provider_owner_token: 1,
            },
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
    fn provider_unregister_rejects_wrong_owner_token() {
        let (mut client, server) = UnixStream::pair().unwrap();
        let runtime = Arc::new(Mutex::new(Runtime::new()));
        let server_runtime = Arc::clone(&runtime);
        let server_thread = thread::spawn(move || handle_client(server, &server_runtime));

        send_hello(&mut client);
        write_frame(&mut client, &Request::ProviderRegister).unwrap();
        let (provider_id, provider_owner_token) =
            registered_provider(read_frame(&mut client).unwrap().unwrap());

        write_frame(
            &mut client,
            &Request::ProviderUnregister {
                provider_id,
                provider_owner_token: provider_owner_token ^ 1,
            },
        )
        .unwrap();
        let response: Response = read_frame(&mut client).unwrap().unwrap();

        assert_eq!(
            response,
            Response::Error(ErrorBody {
                code: ErrorCode::Permission,
                message: "permission denied".to_owned(),
            })
        );

        drop(client);
        server_thread.join().unwrap().unwrap();
    }

    #[test]
    fn promise_commit_mutates_runtime() {
        let (mut client, server) = UnixStream::pair().unwrap();
        let runtime = Arc::new(Mutex::new(Runtime::new()));
        let server_state = mounted_state(Arc::clone(&runtime));
        let server_thread = thread::spawn(move || handle_client_with_state(server, &server_state));

        send_hello(&mut client);
        write_frame(&mut client, &Request::ProviderRegister).unwrap();
        let (provider_id, provider_owner_token) =
            registered_provider(read_frame(&mut client).unwrap().unwrap());

        write_frame(
            &mut client,
            &Request::PromiseCommit(
                sample_commit_request(provider_id, provider_owner_token).into_body(),
            ),
        )
        .unwrap();
        let response: Response = read_frame(&mut client).unwrap().unwrap();
        assert_eq!(
            response,
            Response::PromiseCommitted(PromiseCommittedBody {
                promise_id: "promise-1".to_owned(),
                visible_path: "/tmp/fuse-promise/promise-1".to_owned(),
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
    fn inspect_reports_committed_runtime_tree() {
        let (mut client, server) = UnixStream::pair().unwrap();
        let runtime = Arc::new(Mutex::new(Runtime::new()));
        let server_state = mounted_state(Arc::clone(&runtime));
        let server_thread = thread::spawn(move || handle_client_with_state(server, &server_state));

        send_hello(&mut client);
        write_frame(&mut client, &Request::ProviderRegister).unwrap();
        let (provider_id, provider_owner_token) =
            registered_provider(read_frame(&mut client).unwrap().unwrap());
        write_frame(
            &mut client,
            &Request::PromiseCommit(
                sample_commit_request(provider_id, provider_owner_token).into_body(),
            ),
        )
        .unwrap();
        let _response: Response = read_frame(&mut client).unwrap().unwrap();

        write_frame(&mut client, &Request::Inspect).unwrap();
        let response: Response = read_frame(&mut client).unwrap().unwrap();

        let Response::Inspect(inspect) = response else {
            panic!("unexpected response: {response:?}");
        };
        assert_eq!(inspect.providers, 1);
        assert_eq!(inspect.promises.len(), 1);
        assert_eq!(inspect.promises[0].promise_id, "promise-1");
        assert_eq!(inspect.promises[0].state, "available");
        assert_eq!(inspect.promises[0].nodes.len(), 3);
        assert!(inspect.encode_text().contains("path=docs/readme.txt"));

        drop(client);
        server_thread.join().unwrap().unwrap();
    }

    #[test]
    fn promise_commit_rejects_unmounted_state_without_mutating_runtime() {
        let (mut client, server) = UnixStream::pair().unwrap();
        let runtime = Arc::new(Mutex::new(Runtime::new()));
        let server_runtime = Arc::clone(&runtime);
        let server_thread = thread::spawn(move || handle_client(server, &server_runtime));

        send_hello(&mut client);
        write_frame(&mut client, &Request::ProviderRegister).unwrap();
        let (provider_id, provider_owner_token) =
            registered_provider(read_frame(&mut client).unwrap().unwrap());

        write_frame(
            &mut client,
            &Request::PromiseCommit(
                sample_commit_request(provider_id, provider_owner_token).into_body(),
            ),
        )
        .unwrap();
        let response: Response = read_frame(&mut client).unwrap().unwrap();

        assert_eq!(
            response,
            Response::Error(ErrorBody {
                code: ErrorCode::Unavailable,
                message: "unavailable".to_owned(),
            })
        );

        drop(client);
        server_thread.join().unwrap().unwrap();
        assert_eq!(runtime.lock().unwrap().promise_count(), 0);
    }

    #[test]
    fn promise_commit_rejects_unknown_provider() {
        let (mut client, server) = UnixStream::pair().unwrap();
        let runtime = Arc::new(Mutex::new(Runtime::new()));
        let server_state = mounted_state(Arc::clone(&runtime));
        let server_thread = thread::spawn(move || handle_client_with_state(server, &server_state));

        send_hello(&mut client);
        write_frame(
            &mut client,
            &Request::PromiseCommit(sample_commit_request(99, 1).into_body()),
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
        assert_eq!(runtime.lock().unwrap().promise_count(), 0);
    }

    #[test]
    fn promise_commit_rejects_wrong_provider_owner_token() {
        let (mut client, server) = UnixStream::pair().unwrap();
        let runtime = Arc::new(Mutex::new(Runtime::new()));
        let server_state = mounted_state(Arc::clone(&runtime));
        let server_thread = thread::spawn(move || handle_client_with_state(server, &server_state));

        send_hello(&mut client);
        write_frame(&mut client, &Request::ProviderRegister).unwrap();
        let (provider_id, provider_owner_token) =
            registered_provider(read_frame(&mut client).unwrap().unwrap());

        write_frame(
            &mut client,
            &Request::PromiseCommit(
                sample_commit_request(provider_id, provider_owner_token ^ 1).into_body(),
            ),
        )
        .unwrap();
        let response: Response = read_frame(&mut client).unwrap().unwrap();

        assert_eq!(
            response,
            Response::Error(ErrorBody {
                code: ErrorCode::Permission,
                message: "permission denied".to_owned(),
            })
        );

        drop(client);
        server_thread.join().unwrap().unwrap();
        assert_eq!(runtime.lock().unwrap().promise_count(), 0);
    }

    #[test]
    fn promise_commit_rejects_invalid_metadata_without_mutating_runtime() {
        let invalid_requests = vec![
            invalid_commit_request("absolute", |request| {
                request.nodes[0].relative_path = "/docs".to_owned();
            }),
            invalid_commit_request("dotdot", |request| {
                request.nodes[0].relative_path = "docs/../bad".to_owned();
            }),
            invalid_commit_request("nul", |request| {
                request.nodes[0].relative_path = "do\0cs".to_owned();
            }),
            invalid_commit_request("duplicate", |request| {
                request.nodes.push(request.nodes[0].clone());
            }),
            invalid_commit_request("missing-parent", |request| {
                request.nodes.remove(0);
            }),
            invalid_commit_request("file-parent", |request| {
                request.nodes[0].kind = PromiseNodeKindBody::File;
            }),
            invalid_commit_request("bad-mode", |request| {
                request.nodes[0].mode = 0o100755;
            }),
            invalid_commit_request("nonzero-dir-size", |request| {
                request.nodes[0].size = 1;
            }),
            invalid_commit_request("negative-mtime", |request| {
                request.nodes[0].mtime_nsec = -1;
            }),
            invalid_commit_request("empty-provider-node", |request| {
                request.nodes[0].provider_node_id.clear();
            }),
        ];

        for (case, request) in invalid_requests {
            let (mut client, server) = UnixStream::pair().unwrap();
            let runtime = Arc::new(Mutex::new(Runtime::new()));
            let server_state = mounted_state(Arc::clone(&runtime));
            let server_thread =
                thread::spawn(move || handle_client_with_state(server, &server_state));

            send_hello(&mut client);
            write_frame(&mut client, &Request::ProviderRegister).unwrap();
            let (provider_id, provider_owner_token) =
                registered_provider(read_frame(&mut client).unwrap().unwrap());

            let mut request = request;
            request.provider_id = provider_id;
            request.provider_owner_token = provider_owner_token;
            write_frame(&mut client, &Request::PromiseCommit(request)).unwrap();
            let response: Response = read_frame(&mut client).unwrap().unwrap();
            match response {
                Response::Error(ErrorBody {
                    code: ErrorCode::InvalidRequest | ErrorCode::NotFound | ErrorCode::AlreadyExists,
                    ..
                }) => {}
                other => panic!("{case}: unexpected commit response: {other:?}"),
            }

            drop(client);
            server_thread.join().unwrap().unwrap();
            assert_eq!(runtime.lock().unwrap().promise_count(), 0, "{case}");
        }
    }

    #[test]
    fn provider_connection_drop_marks_provider_disconnected() {
        let (mut client, server) = UnixStream::pair().unwrap();
        let runtime = Arc::new(Mutex::new(Runtime::new()));
        let server_runtime = Arc::clone(&runtime);
        let server_thread = thread::spawn(move || handle_client(server, &server_runtime));

        send_hello(&mut client);
        write_frame(&mut client, &Request::ProviderRegister).unwrap();
        let (provider_id, _) = registered_provider(read_frame(&mut client).unwrap().unwrap());

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
        let server_state = mounted_state(Arc::clone(&runtime));
        let server_thread = thread::spawn(move || handle_client_with_state(server, &server_state));

        send_hello(&mut client);
        write_frame(&mut client, &Request::ProviderRegister).unwrap();
        let (provider_id, provider_owner_token) =
            registered_provider(read_frame(&mut client).unwrap().unwrap());
        write_frame(
            &mut client,
            &Request::PromiseCommit(
                sample_commit_request(provider_id, provider_owner_token).into_body(),
            ),
        )
        .unwrap();
        let response: Response = read_frame(&mut client).unwrap().unwrap();
        assert_eq!(
            response,
            Response::PromiseCommitted(PromiseCommittedBody {
                promise_id: "promise-1".to_owned(),
                visible_path: "/tmp/fuse-promise/promise-1".to_owned(),
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
            let (stream, _) = listener.accept().unwrap();
            handle_client(stream, &server_runtime).unwrap();
        });

        let provider = connect_provider(&socket_path).unwrap();
        let provider_id = provider.provider_id();
        provider.unregister().unwrap();

        server_thread.join().unwrap();
        let provider_id = fuse_promise_runtime::ProviderId::from_raw(provider_id).unwrap();
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
        let (provider_id, _) =
            registered_provider(read_frame(&mut provider.stream).unwrap().unwrap());

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
        let (provider_one_id, _) =
            registered_provider(read_frame(&mut provider_one.stream).unwrap().unwrap());
        send_hello(&mut provider_two.stream);
        write_frame(&mut provider_two.stream, &Request::ProviderRegister).unwrap();
        let (_provider_two_id, _) =
            registered_provider(read_frame(&mut provider_two.stream).unwrap().unwrap());

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
        let server_state = mounted_state(Arc::clone(&runtime));
        let server_thread = thread::spawn(move || {
            let mut children = Vec::new();
            for _ in 0..2 {
                let (stream, _) = listener.accept().unwrap();
                let state = server_state.clone();
                children.push(thread::spawn(move || {
                    handle_client_with_state(stream, &state).unwrap();
                }));
            }
            for child in children {
                child.join().unwrap();
            }
        });

        let provider = connect_provider(&socket_path).unwrap();
        let response = commit_promise(
            &socket_path,
            sample_commit_request(provider.provider_id(), provider.provider_owner_token()),
        )
        .unwrap();

        assert_eq!(response.promise_id, "promise-1");
        assert_eq!(
            response.visible_path,
            PathBuf::from("/tmp/fuse-promise/promise-1")
        );
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
    fn promise_commit_request_from_builder_omits_root_node() {
        let provider_id = fuse_promise_runtime::ProviderId::from_raw(7).unwrap();
        let mut builder = PromiseBuilder::new(provider_id);
        builder
            .add_dir("docs", NodeAttr::new(0o755, 0, 0), "remote-dir-1")
            .unwrap();
        builder
            .add_file(
                "docs/readme.txt",
                NodeAttr::new(0o644, 12, 123),
                "remote-file-1",
            )
            .unwrap();

        let request = PromiseCommitRequest::from_builder_with_owner_token(&builder, 42);

        assert_eq!(request.provider_id, 7);
        assert_eq!(request.provider_owner_token, 42);
        assert_eq!(request.nodes.len(), 2);
        assert_eq!(request.nodes[0].kind, PromiseNodeKind::Directory);
        assert_eq!(request.nodes[0].relative_path, "docs");
        assert_eq!(request.nodes[1].kind, PromiseNodeKind::File);
        assert_eq!(request.nodes[1].relative_path, "docs/readme.txt");
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

    #[test]
    fn materialized_chunk_reads_local_ranges_and_rejects_short_backing_file() {
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("readme.txt");
        fs::write(&source_path, b"hello from fuse-promise\n").unwrap();

        let bytes = read_local_materialized_chunk(&source_path, 6, 4).unwrap();
        assert_eq!(bytes, b"from");

        let error = read_local_materialized_chunk(&source_path, 20, 8).unwrap_err();
        assert_eq!(error, Status::Io);
    }

    #[test]
    fn materialize_existing_target_is_not_owned_for_cleanup() {
        let dir = tempfile::tempdir().unwrap();
        let target_path = dir.path().join("readme.txt");
        fs::write(&target_path, b"existing").unwrap();
        let state = IpcState::new(Arc::new(Mutex::new(Runtime::new())));
        let entry = MaterializeEntryPlan {
            relative_path: "docs/readme.txt".to_owned(),
            target_path: target_path.clone(),
            kind: NodeKind::File,
            size: 1,
            mode: 0o644,
            mtime_nsec: 0,
        };

        let error = write_materialized_file(&state, "promise-1", &entry).unwrap_err();

        assert_eq!(error.status, Status::AlreadyExists);
        assert_eq!(error.created_target, None);
        assert_eq!(fs::read(&target_path).unwrap(), b"existing");
    }

    #[test]
    fn materialize_cleanup_removes_only_matching_target_identity() {
        let dir = tempfile::tempdir().unwrap();
        let target_path = dir.path().join("readme.txt");
        let original_path = dir.path().join("original-readme.txt");
        fs::write(&target_path, b"ours").unwrap();
        let original_identity =
            FileIdentity::from_metadata(&fs::symlink_metadata(&target_path).unwrap());

        fs::rename(&target_path, &original_path).unwrap();
        fs::write(&target_path, b"external").unwrap();

        cleanup_created_targets(&[CreatedTarget {
            path: target_path.clone(),
            identity: original_identity,
            kind: MaterializeTargetKind::File,
        }]);
        assert_eq!(fs::read(&target_path).unwrap(), b"external");
        assert_eq!(fs::read(&original_path).unwrap(), b"ours");

        cleanup_created_targets(&[CreatedTarget {
            path: original_path.clone(),
            identity: original_identity,
            kind: MaterializeTargetKind::File,
        }]);
        assert!(!original_path.exists());
    }

    #[test]
    fn materialize_cleanup_restores_directory_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let root_path = dir.path().join("docs");
        let child_path = root_path.join("guides");
        let file_path = child_path.join("setup.txt");
        fs::create_dir(&root_path).unwrap();
        fs::create_dir(&child_path).unwrap();
        fs::write(&file_path, b"setup").unwrap();

        let root = CreatedTarget {
            path: root_path.clone(),
            identity: FileIdentity::from_metadata(&fs::symlink_metadata(&root_path).unwrap()),
            kind: MaterializeTargetKind::Directory,
        };
        let child = CreatedTarget {
            path: child_path.clone(),
            identity: FileIdentity::from_metadata(&fs::symlink_metadata(&child_path).unwrap()),
            kind: MaterializeTargetKind::Directory,
        };
        let file = CreatedTarget {
            path: file_path.clone(),
            identity: FileIdentity::from_metadata(&fs::symlink_metadata(&file_path).unwrap()),
            kind: MaterializeTargetKind::File,
        };
        fs::set_permissions(&root_path, fs::Permissions::from_mode(0o555)).unwrap();
        fs::set_permissions(&child_path, fs::Permissions::from_mode(0o555)).unwrap();

        cleanup_created_targets(&[root, child, file]);

        assert!(!root_path.exists());
    }

    #[test]
    fn provider_gone_error_preserves_io_kind() {
        let error = error_to_io(ErrorBody {
            code: ErrorCode::ProviderGone,
            message: "provider gone".to_owned(),
        });

        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    }

    #[test]
    fn directory_materialize_reports_structured_partial_failure() {
        let mut runtime = Runtime::new();
        let provider = runtime.register_provider();
        let mut builder = PromiseBuilder::new(provider);
        builder
            .add_dir("docs", NodeAttr::new(0o755, 0, 0), "remote-dir-1")
            .unwrap();
        builder
            .add_file(
                "docs/readme.txt",
                NodeAttr::new(0o644, 12, 0),
                "remote-file-1",
            )
            .unwrap();
        runtime.commit_promise(builder).unwrap();
        let state = mounted_state(Arc::new(Mutex::new(runtime)));
        let target_dir = tempfile::tempdir().unwrap();

        let error = materialize_node_in_state(
            &state,
            MaterializeBody {
                source_path: "/tmp/fuse-promise/promise-1/docs".to_owned(),
                target_dir: target_dir.path().to_string_lossy().into_owned(),
                conflict_policy: MaterializeConflictPolicyBody::Fail,
            },
        )
        .unwrap_err();

        let progress = error.progress.expect("partial progress should be reported");
        assert_eq!(progress.target_path, target_dir.path().join("docs"));
        assert_eq!(progress.directories_created, 1);
        assert_eq!(progress.files_written, 0);
        assert_eq!(progress.bytes_written, 0);
        assert!(!target_dir.path().join("docs").exists());
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

    fn registered_provider(response: Response) -> (u64, u128) {
        match response {
            Response::ProviderRegistered {
                provider_id,
                provider_owner_token,
            } => (provider_id, provider_owner_token),
            other => panic!("unexpected provider response: {other:?}"),
        }
    }

    fn mounted_state(runtime: Arc<Mutex<Runtime>>) -> IpcState {
        let state = IpcState::new(runtime);
        state
            .set_mount_status(IpcMountStatus::commit_ready(PathBuf::from(
                "/tmp/fuse-promise",
            )))
            .unwrap();
        state
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

    fn xdg_runtime_env_lock() -> TestMutexGuard<'static, ()> {
        static LOCK: OnceLock<TestMutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| TestMutex::new(())).lock().unwrap()
    }

    fn sample_commit_request(provider_id: u64, provider_owner_token: u128) -> PromiseCommitRequest {
        PromiseCommitRequest {
            provider_id,
            provider_owner_token,
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

    fn invalid_commit_request(
        name: &'static str,
        mutate: impl FnOnce(&mut PromiseCommitBody),
    ) -> (&'static str, PromiseCommitBody) {
        let mut request = sample_commit_request(1, 1).into_body();
        mutate(&mut request);
        (name, request)
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
