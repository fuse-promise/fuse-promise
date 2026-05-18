use fuse_promise_ipc::{IpcMountStatus, IpcState};
#[cfg(feature = "fuse-mount")]
use fuse_promise_ipc::{ProviderReadRequest, ProviderReadStatus, MAX_PROVIDER_READ_LEN};
#[cfg(feature = "fuse-mount")]
use fuse_promise_runtime::{
    prepare_mount_dir, DirectoryEntry, NodeKind, PromiseNode, ReadPlan, RuntimeEntry, Status,
};
#[cfg(feature = "fuse-mount")]
use std::ffi::OsStr;
#[cfg(feature = "fuse-mount")]
use std::fs;
use std::io;
#[cfg(feature = "fuse-mount")]
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};

#[cfg(feature = "fuse-mount")]
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[cfg(feature = "fuse-mount")]
pub struct FuseMount {
    session: Option<fuser::BackgroundSession>,
}

#[cfg(not(feature = "fuse-mount"))]
pub struct FuseMount;

#[cfg(feature = "fuse-mount")]
struct PromiseFilesystem {
    state: IpcState,
}

#[cfg(feature = "fuse-mount")]
enum FuseReadPlan {
    Provider {
        request: ProviderReadRequest,
        response_offset: u32,
        response_length: u32,
    },
    Materialized {
        path: PathBuf,
        offset: u64,
        length: u32,
    },
    Cached(Vec<u8>),
}

#[cfg(feature = "fuse-mount")]
const TTL: Duration = Duration::from_secs(1);

#[cfg(feature = "fuse-mount")]
impl fuser::Filesystem for PromiseFilesystem {
    fn lookup(
        &self,
        req: &fuser::Request,
        parent: fuser::INodeNo,
        name: &OsStr,
        reply: fuser::ReplyEntry,
    ) {
        let Some(name) = name.to_str() else {
            reply.error(fuser::Errno::EINVAL);
            return;
        };

        match self
            .state
            .runtime()
            .lock()
            .map_err(|_| fuser::Errno::EIO)
            .and_then(|runtime| {
                runtime
                    .lookup_child(u64::from(parent), name)
                    .map_err(status_to_errno)
            }) {
            Ok(entry) => reply.entry(
                &TTL,
                &entry_attr(&entry, req.uid(), req.gid()),
                fuser::Generation(0),
            ),
            Err(errno) => reply.error(errno),
        }
    }

    fn getattr(
        &self,
        req: &fuser::Request,
        ino: fuser::INodeNo,
        _fh: Option<fuser::FileHandle>,
        reply: fuser::ReplyAttr,
    ) {
        match self
            .state
            .runtime()
            .lock()
            .map_err(|_| fuser::Errno::EIO)
            .and_then(|runtime| {
                runtime
                    .lookup_inode(u64::from(ino))
                    .map_err(status_to_errno)
            }) {
            Ok(entry) => reply.attr(&TTL, &entry_attr(&entry, req.uid(), req.gid())),
            Err(errno) => reply.error(errno),
        }
    }

    fn readdir(
        &self,
        _req: &fuser::Request,
        ino: fuser::INodeNo,
        _fh: fuser::FileHandle,
        offset: u64,
        mut reply: fuser::ReplyDirectory,
    ) {
        let entries = match self.directory_entries(u64::from(ino)) {
            Ok(entries) => entries,
            Err(errno) => {
                reply.error(errno);
                return;
            }
        };

        for (index, entry) in entries.into_iter().enumerate().skip(offset as usize) {
            if reply.add(
                fuser::INodeNo(entry.inode),
                (index + 1) as u64,
                node_kind_to_file_type(entry.kind),
                entry.name,
            ) {
                break;
            }
        }
        reply.ok();
    }

    fn open(
        &self,
        _req: &fuser::Request,
        ino: fuser::INodeNo,
        flags: fuser::OpenFlags,
        reply: fuser::ReplyOpen,
    ) {
        if flags.acc_mode() != fuser::OpenAccMode::O_RDONLY {
            reply.error(fuser::Errno::EROFS);
            return;
        }

        match self
            .state
            .runtime()
            .lock()
            .map_err(|_| fuser::Errno::EIO)
            .and_then(|runtime| {
                runtime
                    .lookup_inode(u64::from(ino))
                    .map_err(status_to_errno)
            }) {
            Ok(RuntimeEntry::PromiseNode { node, .. }) if node.kind == NodeKind::File => {
                reply.opened(
                    fuser::FileHandle(u64::from(ino)),
                    fuser::FopenFlags::FOPEN_DIRECT_IO,
                );
            }
            Ok(RuntimeEntry::PromiseNode { .. }) | Ok(RuntimeEntry::MountRoot) => {
                reply.error(fuser::Errno::EISDIR);
            }
            Err(errno) => reply.error(errno),
        }
    }

    fn read(
        &self,
        _req: &fuser::Request,
        ino: fuser::INodeNo,
        _fh: fuser::FileHandle,
        offset: u64,
        size: u32,
        _flags: fuser::OpenFlags,
        _lock_owner: Option<fuser::LockOwner>,
        reply: fuser::ReplyData,
    ) {
        let size = size.min(MAX_PROVIDER_READ_LEN);
        let read_plan = match self.plan_read(u64::from(ino), offset, size) {
            Ok(None) => {
                reply.data(&[]);
                return;
            }
            Ok(Some(read_plan)) => read_plan,
            Err(errno) => {
                reply.error(errno);
                return;
            }
        };

        match read_plan {
            FuseReadPlan::Provider {
                request,
                response_offset,
                response_length,
            } => match self.state.route_provider_read(request.clone()) {
                Ok(response) if response.status == ProviderReadStatus::Ok => {
                    match self.record_cached_read(&request, &response.bytes) {
                        Ok(()) => {
                            self.prefetch_after_provider_read(&request, response.bytes.len());
                            match provider_reply_bytes(
                                &response.bytes,
                                response_offset,
                                response_length,
                            ) {
                                Ok(bytes) => reply.data(bytes),
                                Err(errno) => reply.error(errno),
                            }
                        }
                        Err(errno) => reply.error(errno),
                    }
                }
                Ok(response) => reply.error(provider_status_to_errno(response.status)),
                Err(error) => reply.error(io_error_to_errno(&error)),
            },
            FuseReadPlan::Materialized {
                path,
                offset,
                length,
            } => match read_materialized_file(&path, offset, length) {
                Ok(bytes) => reply.data(&bytes),
                Err(errno) => reply.error(errno),
            },
            FuseReadPlan::Cached(bytes) => reply.data(&bytes),
        }
    }

    fn release(
        &self,
        _req: &fuser::Request,
        _ino: fuser::INodeNo,
        _fh: fuser::FileHandle,
        _flags: fuser::OpenFlags,
        _lock_owner: Option<fuser::LockOwner>,
        _flush: bool,
        reply: fuser::ReplyEmpty,
    ) {
        reply.ok();
    }
}

#[cfg(feature = "fuse-mount")]
impl PromiseFilesystem {
    fn directory_entries(&self, inode: u64) -> Result<Vec<DirectoryEntry>, fuser::Errno> {
        let runtime_state = self.state.runtime();
        let runtime = runtime_state.lock().map_err(|_| fuser::Errno::EIO)?;
        let entry = runtime.lookup_inode(inode).map_err(status_to_errno)?;
        let parent_inode = parent_inode(&runtime, &entry)?;
        let mut entries = vec![
            DirectoryEntry {
                name: ".".to_owned(),
                inode: entry.inode(),
                kind: NodeKind::Directory,
            },
            DirectoryEntry {
                name: "..".to_owned(),
                inode: parent_inode,
                kind: NodeKind::Directory,
            },
        ];
        entries.extend(runtime.read_dir(inode).map_err(status_to_errno)?);
        Ok(entries)
    }

    fn plan_read(
        &self,
        inode: u64,
        offset: u64,
        size: u32,
    ) -> Result<Option<FuseReadPlan>, fuser::Errno> {
        let runtime_state = self.state.runtime();
        let runtime = runtime_state.lock().map_err(|_| fuser::Errno::EIO)?;
        let RuntimeEntry::PromiseNode { promise_id, node } =
            runtime.lookup_inode(inode).map_err(status_to_errno)?
        else {
            return Err(fuser::Errno::EISDIR);
        };
        let read_plan = runtime
            .plan_read(&promise_id, &node.relative_path, offset, size)
            .map_err(status_to_errno)?;

        match read_plan {
            ReadPlan::Eof => Ok(None),
            ReadPlan::Request(plan) => {
                let plan = runtime
                    .plan_coalesced_provider_read(&plan)
                    .map_err(status_to_errno)?;
                Ok(Some(FuseReadPlan::Provider {
                    request: ProviderReadRequest {
                        request_id: self.state.next_provider_read_request_id(),
                        provider_id: plan.provider.provider_id.raw(),
                        promise_id: plan.provider.promise_id,
                        relative_path: plan.provider.relative_path,
                        provider_node_id: plan.provider.provider_node_id,
                        offset: plan.provider.offset,
                        length: plan.provider.length,
                    },
                    response_offset: plan.response_offset,
                    response_length: plan.response_length,
                }))
            }
            ReadPlan::Materialized(plan) => Ok(Some(FuseReadPlan::Materialized {
                path: plan.path,
                offset: plan.offset,
                length: plan.length,
            })),
            ReadPlan::Cached(plan) => Ok(Some(FuseReadPlan::Cached(plan.bytes))),
        }
    }

    fn record_cached_read(
        &self,
        request: &ProviderReadRequest,
        bytes: &[u8],
    ) -> Result<(), fuser::Errno> {
        let runtime_state = self.state.runtime();
        let mut runtime = runtime_state.lock().map_err(|_| fuser::Errno::EIO)?;
        runtime
            .record_cached_read(
                &request.promise_id,
                &request.relative_path,
                request.offset,
                bytes,
            )
            .map_err(status_to_errno)
    }

    fn prefetch_after_provider_read(&self, request: &ProviderReadRequest, bytes_read: usize) {
        if bytes_read != request.length as usize {
            return;
        }
        let Ok(bytes_read) = u32::try_from(bytes_read) else {
            return;
        };

        let prefetch_plan = {
            let runtime_state = self.state.runtime();
            let Ok(runtime) = runtime_state.lock() else {
                return;
            };
            runtime.plan_sequential_prefetch(
                &request.promise_id,
                &request.relative_path,
                request.offset,
                bytes_read,
            )
        };
        let Ok(Some(plan)) = prefetch_plan else {
            return;
        };

        let prefetch_request = ProviderReadRequest {
            request_id: self.state.next_provider_read_request_id(),
            provider_id: plan.provider_id.raw(),
            promise_id: plan.promise_id,
            relative_path: plan.relative_path,
            provider_node_id: plan.provider_node_id,
            offset: plan.offset,
            length: plan.length,
        };
        let Ok(response) = self.state.route_provider_read(prefetch_request.clone()) else {
            return;
        };
        if response.status == ProviderReadStatus::Ok {
            let _ = self.record_cached_read(&prefetch_request, &response.bytes);
        }
    }
}

#[cfg(feature = "fuse-mount")]
fn provider_reply_bytes(
    bytes: &[u8],
    response_offset: u32,
    response_length: u32,
) -> Result<&[u8], fuser::Errno> {
    let start = response_offset as usize;
    let length = response_length as usize;
    if start == 0 && bytes.len() <= length {
        return Ok(bytes);
    }

    let end = start.checked_add(length).ok_or(fuser::Errno::EIO)?;
    if end > bytes.len() {
        return Err(fuser::Errno::EIO);
    }

    Ok(&bytes[start..end])
}

#[cfg(feature = "fuse-mount")]
fn read_materialized_file(path: &Path, offset: u64, length: u32) -> Result<Vec<u8>, fuser::Errno> {
    let file = fs::File::open(path).map_err(|error| io_error_to_errno(&error))?;
    let mut bytes = vec![0_u8; length as usize];
    let read = file
        .read_at(&mut bytes, offset)
        .map_err(|error| io_error_to_errno(&error))?;
    if read != bytes.len() {
        return Err(fuser::Errno::EIO);
    }
    bytes.truncate(read);
    Ok(bytes)
}

#[cfg(feature = "fuse-mount")]
pub fn start(mount_path: &Path, state: IpcState) -> io::Result<Option<FuseMount>> {
    prepare_mount_dir(mount_path).map_err(status_to_io)?;

    let mut config = fuser::Config::default();
    config.mount_options = vec![
        fuser::MountOption::FSName("fuse-promise".to_owned()),
        fuser::MountOption::Subtype("fuse-promise".to_owned()),
        fuser::MountOption::RO,
        fuser::MountOption::DefaultPermissions,
        fuser::MountOption::NoDev,
        fuser::MountOption::NoSuid,
    ];
    config.n_threads = Some(1);

    let filesystem = PromiseFilesystem { state };
    let session = fuser::spawn_mount2(filesystem, mount_path, &config)?;
    Ok(Some(FuseMount {
        session: Some(session),
    }))
}

#[cfg(not(feature = "fuse-mount"))]
pub fn start(_mount_path: &Path, _state: IpcState) -> io::Result<Option<FuseMount>> {
    Ok(None)
}

pub fn mount_status(mount_path: &Path, mount: &Option<FuseMount>) -> IpcMountStatus {
    if mount.is_some() {
        IpcMountStatus::commit_ready(mount_path.to_path_buf())
    } else {
        disabled_mount_status(mount_path.to_path_buf())
    }
}

#[cfg(feature = "fuse-mount")]
fn disabled_mount_status(_mount_path: PathBuf) -> IpcMountStatus {
    IpcMountStatus::not_mounted()
}

#[cfg(not(feature = "fuse-mount"))]
fn disabled_mount_status(mount_path: PathBuf) -> IpcMountStatus {
    IpcMountStatus::disabled(mount_path)
}

#[cfg(feature = "fuse-mount")]
impl Drop for FuseMount {
    fn drop(&mut self) {
        if let Some(session) = self.session.take() {
            let _ = session.umount_and_join();
        }
    }
}

#[cfg(feature = "fuse-mount")]
fn parent_inode(
    runtime: &fuse_promise_runtime::Runtime,
    entry: &RuntimeEntry,
) -> Result<u64, fuser::Errno> {
    match entry {
        RuntimeEntry::MountRoot => Ok(fuse_promise_runtime::FUSE_ROOT_INODE),
        RuntimeEntry::PromiseNode { promise_id, node } => {
            let Some(parent_path) = node.parent_path.as_deref() else {
                return Ok(fuse_promise_runtime::FUSE_ROOT_INODE);
            };
            runtime
                .promise(promise_id)
                .and_then(|tree| tree.nodes.get(parent_path))
                .map(|parent| parent.inode)
                .ok_or(fuser::Errno::ENOENT)
        }
    }
}

#[cfg(feature = "fuse-mount")]
fn entry_attr(entry: &RuntimeEntry, uid: u32, gid: u32) -> fuser::FileAttr {
    match entry {
        RuntimeEntry::MountRoot => fuser::FileAttr {
            ino: fuser::INodeNo(fuse_promise_runtime::FUSE_ROOT_INODE),
            size: 0,
            blocks: 0,
            atime: UNIX_EPOCH,
            mtime: UNIX_EPOCH,
            ctime: UNIX_EPOCH,
            crtime: UNIX_EPOCH,
            kind: fuser::FileType::Directory,
            perm: 0o755,
            nlink: 2,
            uid,
            gid,
            rdev: 0,
            blksize: 4096,
            flags: 0,
        },
        RuntimeEntry::PromiseNode { node, .. } => node_attr(node, uid, gid),
    }
}

#[cfg(feature = "fuse-mount")]
fn node_attr(node: &PromiseNode, uid: u32, gid: u32) -> fuser::FileAttr {
    fuser::FileAttr {
        ino: fuser::INodeNo(node.inode),
        size: node.attr.size,
        blocks: node.attr.size.div_ceil(512),
        atime: mtime(node.attr.mtime_nsec),
        mtime: mtime(node.attr.mtime_nsec),
        ctime: mtime(node.attr.mtime_nsec),
        crtime: mtime(node.attr.mtime_nsec),
        kind: node_kind_to_file_type(node.kind),
        perm: (node.attr.mode & 0o7777) as u16,
        nlink: if node.kind == NodeKind::Directory {
            2
        } else {
            1
        },
        uid,
        gid,
        rdev: 0,
        blksize: 4096,
        flags: 0,
    }
}

#[cfg(feature = "fuse-mount")]
fn mtime(mtime_nsec: i64) -> SystemTime {
    if mtime_nsec >= 0 {
        UNIX_EPOCH + Duration::from_nanos(mtime_nsec as u64)
    } else {
        UNIX_EPOCH
            .checked_sub(Duration::from_nanos(mtime_nsec.saturating_abs() as u64))
            .unwrap_or(UNIX_EPOCH)
    }
}

#[cfg(feature = "fuse-mount")]
fn node_kind_to_file_type(kind: NodeKind) -> fuser::FileType {
    match kind {
        NodeKind::File => fuser::FileType::RegularFile,
        NodeKind::Directory => fuser::FileType::Directory,
    }
}

#[cfg(feature = "fuse-mount")]
fn status_to_errno(status: Status) -> fuser::Errno {
    match status {
        Status::Ok => fuser::Errno::EIO,
        Status::InvalidArgument | Status::VersionMismatch => fuser::Errno::EINVAL,
        Status::Unavailable | Status::ProviderGone | Status::Io => fuser::Errno::EIO,
        Status::Permission => fuser::Errno::EACCES,
        Status::NotFound => fuser::Errno::ENOENT,
        Status::AlreadyExists => fuser::Errno::EEXIST,
        Status::Timeout => fuser::Errno::ETIMEDOUT,
        Status::Cancelled => fuser::Errno::ECANCELED,
    }
}

#[cfg(feature = "fuse-mount")]
fn provider_status_to_errno(status: ProviderReadStatus) -> fuser::Errno {
    match status {
        ProviderReadStatus::Ok => fuser::Errno::EIO,
        ProviderReadStatus::InvalidArgument => fuser::Errno::EINVAL,
        ProviderReadStatus::Permission => fuser::Errno::EACCES,
        ProviderReadStatus::NotFound => fuser::Errno::ENOENT,
        ProviderReadStatus::ProviderGone | ProviderReadStatus::Io => fuser::Errno::EIO,
        ProviderReadStatus::Timeout => fuser::Errno::ETIMEDOUT,
        ProviderReadStatus::Cancelled => fuser::Errno::ECANCELED,
    }
}

#[cfg(feature = "fuse-mount")]
fn io_error_to_errno(error: &io::Error) -> fuser::Errno {
    match error.kind() {
        io::ErrorKind::InvalidInput | io::ErrorKind::InvalidData => fuser::Errno::EINVAL,
        io::ErrorKind::NotFound => fuser::Errno::ENOENT,
        io::ErrorKind::PermissionDenied => fuser::Errno::EACCES,
        io::ErrorKind::TimedOut => fuser::Errno::ETIMEDOUT,
        _ => fuser::Errno::EIO,
    }
}

#[cfg(feature = "fuse-mount")]
fn status_to_io(status: Status) -> io::Error {
    match status {
        Status::InvalidArgument => io::Error::new(io::ErrorKind::InvalidInput, status.as_str()),
        Status::Permission => io::Error::new(io::ErrorKind::PermissionDenied, status.as_str()),
        Status::AlreadyExists => io::Error::new(io::ErrorKind::AlreadyExists, status.as_str()),
        Status::NotFound => io::Error::new(io::ErrorKind::NotFound, status.as_str()),
        _ => io::Error::other(status.as_str()),
    }
}
