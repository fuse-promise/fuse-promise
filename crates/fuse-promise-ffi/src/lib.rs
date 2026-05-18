#![allow(non_camel_case_types)]

use fuse_promise_ipc::{
    commit_promise, connect_provider, materialize_file, MaterializeConflictPolicy,
    MaterializeRequest, PromiseCommitRequest, ProviderConnection, ProviderReadRequest,
    ProviderReadResponse, ProviderReadStatus,
};
use fuse_promise_runtime::{
    default_control_socket_path, default_mount_path, validate_runtime_dir_path, NodeAttr,
    PromiseBuilder, ProviderId, Status, API_VERSION,
};
use std::ffi::{CStr, CString};
use std::fs;
use std::io;
use std::os::raw::{c_char, c_void};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::ptr;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

#[allow(non_camel_case_types)]
pub type fp_status_t = u32;

pub const FP_OK: fp_status_t = 0;
pub const FP_ERR_INVALID_ARGUMENT: fp_status_t = 1;
pub const FP_ERR_UNAVAILABLE: fp_status_t = 2;
pub const FP_ERR_PERMISSION: fp_status_t = 3;
pub const FP_ERR_NOT_FOUND: fp_status_t = 4;
pub const FP_ERR_ALREADY_EXISTS: fp_status_t = 5;
pub const FP_ERR_PROVIDER_GONE: fp_status_t = 6;
pub const FP_ERR_IO: fp_status_t = 7;
pub const FP_ERR_TIMEOUT: fp_status_t = 8;
pub const FP_ERR_CANCELLED: fp_status_t = 9;
pub const FP_ERR_VERSION_MISMATCH: fp_status_t = 10;

#[repr(C)]
pub struct fp_context_options_t {
    pub struct_size: u32,
    pub api_version: u32,
    pub runtime_dir: *const c_char,
}

#[repr(C)]
pub struct fp_read_request_t {
    pub promise_id: *const c_char,
    pub node_id: *const c_char,
    pub relative_path: *const c_char,
    pub offset: u64,
    pub length: usize,
}

#[repr(C)]
pub struct fp_read_response_t {
    pub buffer: *mut u8,
    pub buffer_len: usize,
    pub bytes_written: usize,
}

pub type fp_provider_read_fn = Option<
    unsafe extern "C" fn(
        request: *const fp_read_request_t,
        response: *mut fp_read_response_t,
        user_data: *mut c_void,
    ) -> fp_status_t,
>;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct fp_provider_ops_t {
    pub struct_size: u32,
    pub read: fp_provider_read_fn,
}

#[repr(C)]
pub struct fp_node_attr_t {
    pub struct_size: u32,
    pub mode: u32,
    pub size: u64,
    pub mtime_nsec: i64,
}

#[allow(non_camel_case_types)]
pub type fp_conflict_policy_t = u32;

pub const FP_CONFLICT_FAIL: fp_conflict_policy_t = 0;
pub const FP_CONFLICT_OVERWRITE: fp_conflict_policy_t = 1;
pub const FP_CONFLICT_RENAME: fp_conflict_policy_t = 2;

#[repr(C)]
pub struct fp_materialize_options_t {
    pub struct_size: u32,
    pub conflict_policy: fp_conflict_policy_t,
}

pub struct fp_context {
    inner: Arc<ContextInner>,
}

pub struct fp_provider {
    inner: Arc<ContextInner>,
    id: ProviderId,
    helper: Option<ProviderHelper>,
}

pub struct fp_promise_builder {
    inner: Arc<ContextInner>,
    builder: Mutex<Option<PromiseBuilder>>,
}

pub enum fp_materialize_job {}

struct ContextInner {
    socket_path: PathBuf,
    _mount_path: PathBuf,
}

struct ProviderHelper {
    shutdown: std::os::unix::net::UnixStream,
    thread: Option<JoinHandle<()>>,
}

#[no_mangle]
pub extern "C" fn fp_status_string(status: fp_status_t) -> *const c_char {
    let bytes: &[u8] = match status {
        FP_OK => b"ok\0",
        FP_ERR_INVALID_ARGUMENT => b"invalid argument\0",
        FP_ERR_UNAVAILABLE => b"unavailable\0",
        FP_ERR_PERMISSION => b"permission denied\0",
        FP_ERR_NOT_FOUND => b"not found\0",
        FP_ERR_ALREADY_EXISTS => b"already exists\0",
        FP_ERR_PROVIDER_GONE => b"provider gone\0",
        FP_ERR_IO => b"io error\0",
        FP_ERR_TIMEOUT => b"timeout\0",
        FP_ERR_CANCELLED => b"cancelled\0",
        FP_ERR_VERSION_MISMATCH => b"version mismatch\0",
        _ => b"unknown status\0",
    };
    bytes.as_ptr().cast()
}

#[no_mangle]
pub unsafe extern "C" fn fp_context_open(
    options: *const fp_context_options_t,
    out_context: *mut *mut fp_context,
) -> fp_status_t {
    ffi_guard(|| unsafe {
        if out_context.is_null() {
            return Err(FP_ERR_INVALID_ARGUMENT);
        }
        *out_context = ptr::null_mut();

        let runtime_paths = runtime_paths_from_options(options)?;
        let context = fp_context {
            inner: Arc::new(ContextInner {
                socket_path: runtime_paths.socket_path,
                _mount_path: runtime_paths.mount_path,
            }),
        };

        *out_context = Box::into_raw(Box::new(context));
        Ok(FP_OK)
    })
}

#[no_mangle]
pub unsafe extern "C" fn fp_context_close(context: *mut fp_context) {
    if !context.is_null() {
        drop(Box::from_raw(context));
    }
}

#[no_mangle]
pub unsafe extern "C" fn fp_provider_register(
    context: *mut fp_context,
    ops: *const fp_provider_ops_t,
    user_data: *mut c_void,
    out_provider: *mut *mut fp_provider,
) -> fp_status_t {
    ffi_guard(|| unsafe {
        if context.is_null() || ops.is_null() || out_provider.is_null() {
            return Err(FP_ERR_INVALID_ARGUMENT);
        }
        *out_provider = ptr::null_mut();

        let struct_size = (*ops).struct_size as usize;
        if struct_size < required_provider_ops_size() {
            return Err(FP_ERR_INVALID_ARGUMENT);
        }
        let read = ops_read(ops);
        if read.is_none() {
            return Err(FP_ERR_INVALID_ARGUMENT);
        }

        let inner = (*context).inner.clone();
        let connection = connect_provider(&inner.socket_path).map_err(io_to_ffi)?;
        let id = ProviderId::from_raw(connection.provider_id()).ok_or(FP_ERR_IO)?;
        let helper = spawn_provider_helper(connection, read, user_data)?;

        let provider = fp_provider {
            inner,
            id,
            helper: Some(helper),
        };

        *out_provider = Box::into_raw(Box::new(provider));
        Ok(FP_OK)
    })
}

#[no_mangle]
pub unsafe extern "C" fn fp_provider_unregister(provider: *mut fp_provider) {
    if provider.is_null() {
        return;
    }

    let mut provider = Box::from_raw(provider);
    if let Some(mut helper) = provider.helper.take() {
        helper.shutdown();
    }
}

#[no_mangle]
pub unsafe extern "C" fn fp_promise_builder_new(
    context: *mut fp_context,
    provider: *mut fp_provider,
    out_builder: *mut *mut fp_promise_builder,
) -> fp_status_t {
    ffi_guard(|| unsafe {
        if context.is_null() || provider.is_null() || out_builder.is_null() {
            return Err(FP_ERR_INVALID_ARGUMENT);
        }
        *out_builder = ptr::null_mut();

        if !Arc::ptr_eq(&(*context).inner, &(*provider).inner) {
            return Err(FP_ERR_INVALID_ARGUMENT);
        }

        let provider_id = (*provider).id;

        let builder = fp_promise_builder {
            inner: (*context).inner.clone(),
            builder: Mutex::new(Some(PromiseBuilder::new(provider_id))),
        };

        *out_builder = Box::into_raw(Box::new(builder));
        Ok(FP_OK)
    })
}

#[no_mangle]
pub unsafe extern "C" fn fp_promise_add_dir(
    builder: *mut fp_promise_builder,
    relative_path: *const c_char,
    attr: *const fp_node_attr_t,
    provider_node_id: *const c_char,
) -> fp_status_t {
    ffi_guard(|| unsafe {
        let builder = builder_mut(builder)?;
        let relative_path = cstr_to_str(relative_path)?;
        let provider_node_id = cstr_to_str(provider_node_id)?;
        let attr = node_attr(attr)?;

        let mut guard = builder.builder.lock().map_err(|_| FP_ERR_IO)?;
        let Some(inner_builder) = guard.as_mut() else {
            return Err(FP_ERR_INVALID_ARGUMENT);
        };

        inner_builder
            .add_dir(relative_path, attr, provider_node_id)
            .map(|_| FP_OK)
            .map_err(status_to_ffi)
    })
}

#[no_mangle]
pub unsafe extern "C" fn fp_promise_add_file(
    builder: *mut fp_promise_builder,
    relative_path: *const c_char,
    attr: *const fp_node_attr_t,
    provider_node_id: *const c_char,
) -> fp_status_t {
    ffi_guard(|| unsafe {
        let builder = builder_mut(builder)?;
        let relative_path = cstr_to_str(relative_path)?;
        let provider_node_id = cstr_to_str(provider_node_id)?;
        let attr = node_attr(attr)?;

        let mut guard = builder.builder.lock().map_err(|_| FP_ERR_IO)?;
        let Some(inner_builder) = guard.as_mut() else {
            return Err(FP_ERR_INVALID_ARGUMENT);
        };

        inner_builder
            .add_file(relative_path, attr, provider_node_id)
            .map(|_| FP_OK)
            .map_err(status_to_ffi)
    })
}

#[no_mangle]
pub unsafe extern "C" fn fp_promise_commit(
    builder: *mut fp_promise_builder,
    out_path: *mut c_char,
    out_path_len: usize,
) -> fp_status_t {
    ffi_guard(|| unsafe {
        let builder = builder_mut(builder)?;
        if out_path.is_null() || out_path_len == 0 {
            return Err(FP_ERR_INVALID_ARGUMENT);
        }
        *out_path = 0;

        if !commit_path_capacity_fits(&builder.inner._mount_path, out_path_len) {
            return Err(FP_ERR_INVALID_ARGUMENT);
        }

        let mut guard = builder.builder.lock().map_err(|_| FP_ERR_IO)?;
        let Some(inner_builder) = guard.as_ref() else {
            return Err(FP_ERR_INVALID_ARGUMENT);
        };

        let request = PromiseCommitRequest::from_builder(inner_builder);
        let response = commit_promise(&builder.inner.socket_path, request).map_err(io_to_ffi)?;
        let visible_path = response.visible_path.to_string_lossy();
        write_c_string(out_path, out_path_len, visible_path.as_ref())?;
        *guard = None;

        Ok(FP_OK)
    })
}

#[no_mangle]
pub unsafe extern "C" fn fp_promise_builder_free(builder: *mut fp_promise_builder) {
    if !builder.is_null() {
        drop(Box::from_raw(builder));
    }
}

#[no_mangle]
pub unsafe extern "C" fn fp_materialize(
    context: *mut fp_context,
    promise_path: *const c_char,
    target_dir: *const c_char,
    options: *const fp_materialize_options_t,
) -> fp_status_t {
    ffi_guard(|| unsafe {
        if context.is_null() {
            return Err(FP_ERR_INVALID_ARGUMENT);
        }
        let context = &*context;
        let promise_path = absolute_client_path(cstr_to_str(promise_path)?).map_err(io_to_ffi)?;
        let target_dir = canonical_target_dir(cstr_to_str(target_dir)?)?;
        let conflict_policy = materialize_options(options)?;

        materialize_file(
            &context.inner.socket_path,
            MaterializeRequest {
                source_path: promise_path,
                target_dir,
                conflict_policy,
            },
        )
        .map_err(io_to_ffi)?;

        Ok(FP_OK)
    })
}

fn ffi_guard(action: impl FnOnce() -> Result<fp_status_t, fp_status_t>) -> fp_status_t {
    match catch_unwind(AssertUnwindSafe(action)) {
        Ok(Ok(status)) => status,
        Ok(Err(status)) => status,
        Err(_) => FP_ERR_IO,
    }
}

fn spawn_provider_helper(
    mut connection: ProviderConnection,
    read: fp_provider_read_fn,
    user_data: *mut c_void,
) -> Result<ProviderHelper, fp_status_t> {
    let shutdown = connection.try_clone_stream().map_err(io_to_ffi)?;
    let Some(read) = read else {
        return Err(FP_ERR_INVALID_ARGUMENT);
    };
    let user_data = user_data as usize;
    let thread = thread::Builder::new()
        .name("fuse-promise-provider".to_owned())
        .spawn(move || {
            let user_data = user_data as *mut c_void;
            while let Ok(Some(request)) = connection.read_provider_read_request() {
                let response = dispatch_provider_read(&request, read, user_data);
                if connection.write_provider_read_response(&response).is_err() {
                    break;
                }
            }
        })
        .map_err(|_| FP_ERR_IO)?;

    Ok(ProviderHelper {
        shutdown,
        thread: Some(thread),
    })
}

impl ProviderHelper {
    fn shutdown(&mut self) {
        let _ = self.shutdown.shutdown(std::net::Shutdown::Both);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for ProviderHelper {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn dispatch_provider_read(
    request: &ProviderReadRequest,
    read: unsafe extern "C" fn(
        request: *const fp_read_request_t,
        response: *mut fp_read_response_t,
        user_data: *mut c_void,
    ) -> fp_status_t,
    user_data: *mut c_void,
) -> ProviderReadResponse {
    let promise_id = match CString::new(request.promise_id.as_str()) {
        Ok(value) => value,
        Err(_) => {
            return provider_read_error(request.request_id, ProviderReadStatus::InvalidArgument)
        }
    };
    let node_id = match CString::new(request.provider_node_id.as_str()) {
        Ok(value) => value,
        Err(_) => {
            return provider_read_error(request.request_id, ProviderReadStatus::InvalidArgument)
        }
    };
    let relative_path = match CString::new(request.relative_path.as_str()) {
        Ok(value) => value,
        Err(_) => {
            return provider_read_error(request.request_id, ProviderReadStatus::InvalidArgument)
        }
    };

    let mut bytes = vec![0_u8; request.length as usize];
    let c_request = fp_read_request_t {
        promise_id: promise_id.as_ptr(),
        node_id: node_id.as_ptr(),
        relative_path: relative_path.as_ptr(),
        offset: request.offset,
        length: request.length as usize,
    };
    let mut c_response = fp_read_response_t {
        buffer: bytes.as_mut_ptr(),
        buffer_len: bytes.len(),
        bytes_written: 0,
    };

    let status = match catch_unwind(AssertUnwindSafe(|| unsafe {
        read(&c_request, &mut c_response, user_data)
    })) {
        Ok(status) => status,
        Err(_) => FP_ERR_IO,
    };

    let status = provider_read_status_from_ffi(status);
    if status != ProviderReadStatus::Ok {
        return provider_read_error(request.request_id, status);
    }
    if c_response.bytes_written > c_response.buffer_len {
        return provider_read_error(request.request_id, ProviderReadStatus::InvalidArgument);
    }

    bytes.truncate(c_response.bytes_written);
    ProviderReadResponse {
        request_id: request.request_id,
        status: ProviderReadStatus::Ok,
        bytes,
    }
}

fn provider_read_error(request_id: u64, status: ProviderReadStatus) -> ProviderReadResponse {
    ProviderReadResponse {
        request_id,
        status,
        bytes: Vec::new(),
    }
}

struct RuntimePaths {
    mount_path: PathBuf,
    socket_path: PathBuf,
}

unsafe fn runtime_paths_from_options(
    options: *const fp_context_options_t,
) -> Result<RuntimePaths, fp_status_t> {
    if options.is_null() {
        return Ok(RuntimePaths {
            mount_path: default_mount_path().map_err(status_to_ffi)?,
            socket_path: default_control_socket_path().map_err(status_to_ffi)?,
        });
    }

    if ((*options).struct_size as usize) < required_context_options_size() {
        return Err(FP_ERR_INVALID_ARGUMENT);
    }
    if (*options).api_version != API_VERSION {
        return Err(FP_ERR_VERSION_MISMATCH);
    }

    let runtime_dir = (*options).runtime_dir;
    if runtime_dir.is_null() {
        return Ok(RuntimePaths {
            mount_path: default_mount_path().map_err(status_to_ffi)?,
            socket_path: default_control_socket_path().map_err(status_to_ffi)?,
        });
    }

    let runtime_dir = PathBuf::from(cstr_to_str(runtime_dir)?);
    validate_runtime_dir_path(&runtime_dir).map_err(status_to_ffi)?;
    Ok(RuntimePaths {
        mount_path: runtime_dir.join("fuse-promise"),
        socket_path: runtime_dir.join("fuse-promise.sock"),
    })
}

unsafe fn cstr_to_str<'a>(value: *const c_char) -> Result<&'a str, fp_status_t> {
    if value.is_null() {
        return Err(FP_ERR_INVALID_ARGUMENT);
    }

    CStr::from_ptr(value)
        .to_str()
        .map_err(|_| FP_ERR_INVALID_ARGUMENT)
}

unsafe fn node_attr(attr: *const fp_node_attr_t) -> Result<NodeAttr, fp_status_t> {
    if attr.is_null() {
        return Err(FP_ERR_INVALID_ARGUMENT);
    }

    if ((*attr).struct_size as usize) < required_node_attr_size() {
        return Err(FP_ERR_INVALID_ARGUMENT);
    }
    let attr = &*attr;
    Ok(NodeAttr::new(attr.mode, attr.size, attr.mtime_nsec))
}

unsafe fn materialize_options(
    options: *const fp_materialize_options_t,
) -> Result<MaterializeConflictPolicy, fp_status_t> {
    if options.is_null() {
        return Ok(MaterializeConflictPolicy::Fail);
    }
    if ((*options).struct_size as usize) < required_materialize_options_size() {
        return Err(FP_ERR_INVALID_ARGUMENT);
    }
    match (*options).conflict_policy {
        FP_CONFLICT_FAIL => Ok(MaterializeConflictPolicy::Fail),
        FP_CONFLICT_OVERWRITE => Ok(MaterializeConflictPolicy::Overwrite),
        FP_CONFLICT_RENAME => Ok(MaterializeConflictPolicy::Rename),
        _ => Err(FP_ERR_INVALID_ARGUMENT),
    }
}

fn absolute_client_path(path: &str) -> io::Result<PathBuf> {
    let path = PathBuf::from(path);
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

fn canonical_target_dir(path: &str) -> Result<PathBuf, fp_status_t> {
    let path = fs::canonicalize(path).map_err(io_to_ffi)?;
    if path.is_dir() {
        Ok(path)
    } else {
        Err(FP_ERR_INVALID_ARGUMENT)
    }
}

fn commit_path_capacity_fits(mount_path: &std::path::Path, out_path_len: usize) -> bool {
    const MAX_PROMISE_ID: &str = "promise-18446744073709551615";
    let max_path = mount_path.join(MAX_PROMISE_ID);
    out_path_len > max_path.to_string_lossy().len()
}

unsafe fn write_c_string(
    out_path: *mut c_char,
    out_path_len: usize,
    value: &str,
) -> Result<(), fp_status_t> {
    if value.as_bytes().contains(&0) || value.len() >= out_path_len {
        return Err(FP_ERR_INVALID_ARGUMENT);
    }

    ptr::copy_nonoverlapping(value.as_ptr(), out_path.cast::<u8>(), value.len());
    *out_path.add(value.len()) = 0;
    Ok(())
}

unsafe fn builder_mut<'a>(
    builder: *mut fp_promise_builder,
) -> Result<&'a fp_promise_builder, fp_status_t> {
    if builder.is_null() {
        return Err(FP_ERR_INVALID_ARGUMENT);
    }

    Ok(&*builder)
}

fn required_context_options_size() -> usize {
    struct_field_end::<fp_context_options_t, *const c_char>(2 * std::mem::size_of::<u32>())
}

fn required_provider_ops_size() -> usize {
    struct_field_end::<fp_provider_ops_t, fp_provider_read_fn>(std::mem::size_of::<u32>())
}

fn required_node_attr_size() -> usize {
    let mode_end = 2 * std::mem::size_of::<u32>();
    let size_end = struct_field_end::<fp_node_attr_t, u64>(mode_end);
    struct_field_end::<fp_node_attr_t, i64>(size_end)
}

fn required_materialize_options_size() -> usize {
    2 * std::mem::size_of::<u32>()
}

fn struct_field_end<T, F>(previous_end: usize) -> usize {
    let _ = std::mem::size_of::<T>();
    let offset = align_up(previous_end, std::mem::align_of::<F>());
    offset + std::mem::size_of::<F>()
}

fn align_up(value: usize, align: usize) -> usize {
    debug_assert!(align.is_power_of_two());
    (value + align - 1) & !(align - 1)
}

unsafe fn ops_read(ops: *const fp_provider_ops_t) -> fp_provider_read_fn {
    let offset = align_up(
        std::mem::size_of::<u32>(),
        std::mem::align_of::<fp_provider_read_fn>(),
    );
    ops.cast::<u8>()
        .add(offset)
        .cast::<fp_provider_read_fn>()
        .read()
}

fn status_to_ffi(status: Status) -> fp_status_t {
    match status {
        Status::Ok => FP_OK,
        Status::InvalidArgument => FP_ERR_INVALID_ARGUMENT,
        Status::Unavailable => FP_ERR_UNAVAILABLE,
        Status::Permission => FP_ERR_PERMISSION,
        Status::NotFound => FP_ERR_NOT_FOUND,
        Status::AlreadyExists => FP_ERR_ALREADY_EXISTS,
        Status::ProviderGone => FP_ERR_PROVIDER_GONE,
        Status::Io => FP_ERR_IO,
        Status::Timeout => FP_ERR_TIMEOUT,
        Status::Cancelled => FP_ERR_CANCELLED,
        Status::VersionMismatch => FP_ERR_VERSION_MISMATCH,
    }
}

fn provider_read_status_from_ffi(status: fp_status_t) -> ProviderReadStatus {
    match status {
        FP_OK => ProviderReadStatus::Ok,
        FP_ERR_INVALID_ARGUMENT | FP_ERR_VERSION_MISMATCH => ProviderReadStatus::InvalidArgument,
        FP_ERR_PERMISSION => ProviderReadStatus::Permission,
        FP_ERR_NOT_FOUND | FP_ERR_ALREADY_EXISTS => ProviderReadStatus::NotFound,
        FP_ERR_PROVIDER_GONE | FP_ERR_UNAVAILABLE => ProviderReadStatus::ProviderGone,
        FP_ERR_TIMEOUT => ProviderReadStatus::Timeout,
        FP_ERR_CANCELLED => ProviderReadStatus::Cancelled,
        FP_ERR_IO => ProviderReadStatus::Io,
        _ => ProviderReadStatus::Io,
    }
}

fn io_to_ffi(error: io::Error) -> fp_status_t {
    match error.kind() {
        io::ErrorKind::InvalidInput | io::ErrorKind::InvalidData => FP_ERR_VERSION_MISMATCH,
        io::ErrorKind::AlreadyExists => FP_ERR_ALREADY_EXISTS,
        io::ErrorKind::NotFound
        | io::ErrorKind::ConnectionRefused
        | io::ErrorKind::AddrNotAvailable => FP_ERR_UNAVAILABLE,
        io::ErrorKind::BrokenPipe => FP_ERR_PROVIDER_GONE,
        io::ErrorKind::PermissionDenied => FP_ERR_PERMISSION,
        io::ErrorKind::TimedOut => FP_ERR_TIMEOUT,
        _ => FP_ERR_IO,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fuse_promise_ipc::{
        read_provider_read_response, serve_state, write_provider_read_request, IpcMountStatus,
        IpcState,
    };
    use fuse_promise_runtime::Runtime;
    use std::ffi::OsString;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixStream;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    #[test]
    fn dispatch_provider_read_calls_c_callback() {
        let request = sample_read_request();

        let response = dispatch_provider_read(&request, test_read_callback, std::ptr::null_mut());

        assert_eq!(response.request_id, request.request_id);
        assert_eq!(response.status, ProviderReadStatus::Ok);
        assert_eq!(response.bytes, b"abc");
    }

    #[test]
    fn provider_helper_dispatches_socket_read_requests() {
        let (provider_stream, mut daemon_stream) = UnixStream::pair().unwrap();
        let connection = ProviderConnection::from_stream_for_test(provider_stream, 1);
        let mut helper =
            spawn_provider_helper(connection, Some(test_read_callback), std::ptr::null_mut())
                .unwrap();
        let request = sample_read_request();

        write_provider_read_request(&mut daemon_stream, &request).unwrap();
        let response = read_provider_read_response(&mut daemon_stream)
            .unwrap()
            .unwrap();

        assert_eq!(response.request_id, request.request_id);
        assert_eq!(response.status, ProviderReadStatus::Ok);
        assert_eq!(response.bytes, b"abc");

        helper.shutdown();
    }

    #[test]
    fn provider_gone_io_maps_to_provider_gone_status() {
        assert_eq!(
            io_to_ffi(io::Error::new(io::ErrorKind::BrokenPipe, "provider gone")),
            FP_ERR_PROVIDER_GONE
        );
    }

    #[test]
    fn public_abi_constants_and_layout_match_header() {
        assert_eq!(API_VERSION, 1);
        assert_eq!(std::mem::size_of::<fp_status_t>(), 4);
        assert_eq!(FP_OK, 0);
        assert_eq!(FP_ERR_INVALID_ARGUMENT, 1);
        assert_eq!(FP_ERR_UNAVAILABLE, 2);
        assert_eq!(FP_ERR_PERMISSION, 3);
        assert_eq!(FP_ERR_NOT_FOUND, 4);
        assert_eq!(FP_ERR_ALREADY_EXISTS, 5);
        assert_eq!(FP_ERR_PROVIDER_GONE, 6);
        assert_eq!(FP_ERR_IO, 7);
        assert_eq!(FP_ERR_TIMEOUT, 8);
        assert_eq!(FP_ERR_CANCELLED, 9);
        assert_eq!(FP_ERR_VERSION_MISMATCH, 10);
        assert_eq!(FP_CONFLICT_FAIL, 0);
        assert_eq!(FP_CONFLICT_OVERWRITE, 1);
        assert_eq!(FP_CONFLICT_RENAME, 2);

        assert_eq!(std::mem::size_of::<fp_context_options_t>(), 16);
        assert_eq!(std::mem::align_of::<fp_context_options_t>(), 8);
        assert_eq!(std::mem::offset_of!(fp_context_options_t, struct_size), 0);
        assert_eq!(std::mem::offset_of!(fp_context_options_t, api_version), 4);
        assert_eq!(std::mem::offset_of!(fp_context_options_t, runtime_dir), 8);

        assert_eq!(std::mem::size_of::<fp_read_request_t>(), 40);
        assert_eq!(std::mem::align_of::<fp_read_request_t>(), 8);
        assert_eq!(std::mem::offset_of!(fp_read_request_t, promise_id), 0);
        assert_eq!(std::mem::offset_of!(fp_read_request_t, node_id), 8);
        assert_eq!(std::mem::offset_of!(fp_read_request_t, relative_path), 16);
        assert_eq!(std::mem::offset_of!(fp_read_request_t, offset), 24);
        assert_eq!(std::mem::offset_of!(fp_read_request_t, length), 32);

        assert_eq!(std::mem::size_of::<fp_read_response_t>(), 24);
        assert_eq!(std::mem::align_of::<fp_read_response_t>(), 8);
        assert_eq!(std::mem::offset_of!(fp_read_response_t, buffer), 0);
        assert_eq!(std::mem::offset_of!(fp_read_response_t, buffer_len), 8);
        assert_eq!(std::mem::offset_of!(fp_read_response_t, bytes_written), 16);

        assert_eq!(std::mem::size_of::<fp_provider_ops_t>(), 16);
        assert_eq!(std::mem::align_of::<fp_provider_ops_t>(), 8);
        assert_eq!(std::mem::offset_of!(fp_provider_ops_t, struct_size), 0);
        assert_eq!(std::mem::offset_of!(fp_provider_ops_t, read), 8);

        assert_eq!(std::mem::size_of::<fp_node_attr_t>(), 24);
        assert_eq!(std::mem::align_of::<fp_node_attr_t>(), 8);
        assert_eq!(std::mem::offset_of!(fp_node_attr_t, struct_size), 0);
        assert_eq!(std::mem::offset_of!(fp_node_attr_t, mode), 4);
        assert_eq!(std::mem::offset_of!(fp_node_attr_t, size), 8);
        assert_eq!(std::mem::offset_of!(fp_node_attr_t, mtime_nsec), 16);

        assert_eq!(std::mem::size_of::<fp_materialize_options_t>(), 8);
        assert_eq!(std::mem::align_of::<fp_materialize_options_t>(), 4);
        assert_eq!(
            std::mem::offset_of!(fp_materialize_options_t, struct_size),
            0
        );
        assert_eq!(
            std::mem::offset_of!(fp_materialize_options_t, conflict_policy),
            4
        );
    }

    #[test]
    fn public_entrypoints_reject_nulls_without_unwinding() {
        let status_string =
            std::panic::catch_unwind(|| fp_status_string(FP_ERR_INVALID_ARGUMENT)).unwrap();
        assert!(!status_string.is_null());

        assert_eq!(
            std::panic::catch_unwind(|| unsafe {
                fp_context_open(std::ptr::null(), std::ptr::null_mut())
            })
            .unwrap(),
            FP_ERR_INVALID_ARGUMENT
        );
        assert!(
            std::panic::catch_unwind(|| unsafe { fp_context_close(std::ptr::null_mut()) }).is_ok()
        );
        assert_eq!(
            std::panic::catch_unwind(|| unsafe {
                fp_provider_register(
                    std::ptr::null_mut(),
                    std::ptr::null(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            })
            .unwrap(),
            FP_ERR_INVALID_ARGUMENT
        );
        assert!(std::panic::catch_unwind(|| unsafe {
            fp_provider_unregister(std::ptr::null_mut())
        })
        .is_ok());
        assert_eq!(
            std::panic::catch_unwind(|| unsafe {
                fp_promise_builder_new(
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            })
            .unwrap(),
            FP_ERR_INVALID_ARGUMENT
        );
        assert_eq!(
            std::panic::catch_unwind(|| unsafe {
                fp_promise_add_dir(
                    std::ptr::null_mut(),
                    std::ptr::null(),
                    std::ptr::null(),
                    std::ptr::null(),
                )
            })
            .unwrap(),
            FP_ERR_INVALID_ARGUMENT
        );
        assert_eq!(
            std::panic::catch_unwind(|| unsafe {
                fp_promise_add_file(
                    std::ptr::null_mut(),
                    std::ptr::null(),
                    std::ptr::null(),
                    std::ptr::null(),
                )
            })
            .unwrap(),
            FP_ERR_INVALID_ARGUMENT
        );
        assert_eq!(
            std::panic::catch_unwind(|| unsafe {
                fp_promise_commit(std::ptr::null_mut(), std::ptr::null_mut(), 0)
            })
            .unwrap(),
            FP_ERR_INVALID_ARGUMENT
        );
        assert!(std::panic::catch_unwind(|| unsafe {
            fp_promise_builder_free(std::ptr::null_mut())
        })
        .is_ok());
        assert_eq!(
            std::panic::catch_unwind(|| unsafe {
                fp_materialize(
                    std::ptr::null_mut(),
                    std::ptr::null(),
                    std::ptr::null(),
                    std::ptr::null(),
                )
            })
            .unwrap(),
            FP_ERR_INVALID_ARGUMENT
        );
    }

    #[test]
    fn promise_commit_unavailable_keeps_builder_retriable() {
        let provider_id = ProviderId::from_raw(1).unwrap();
        let mut pending_builder = PromiseBuilder::new(provider_id);
        pending_builder
            .add_dir("docs", NodeAttr::new(0o755, 0, 0), "remote-dir-1")
            .unwrap();
        pending_builder
            .add_file(
                "docs/readme.txt",
                NodeAttr::new(0o644, 12, 0),
                "remote-file-1",
            )
            .unwrap();
        let inner = Arc::new(ContextInner {
            socket_path: std::env::temp_dir().join("fuse-promise-missing.sock"),
            _mount_path: PathBuf::from("/tmp/fuse-promise"),
        });
        let mut builder = fp_promise_builder {
            inner,
            builder: Mutex::new(Some(pending_builder)),
        };
        let mut out_path = [1_i8; 512];

        let status =
            unsafe { fp_promise_commit(&mut builder, out_path.as_mut_ptr(), out_path.len()) };

        assert_eq!(status, FP_ERR_UNAVAILABLE);
        assert_eq!(out_path[0], 0);
        assert!(builder.builder.lock().unwrap().is_some());
    }

    #[test]
    fn promise_commit_success_returns_visible_path() {
        let runtime_dir = unique_runtime_dir();
        fs::create_dir(&runtime_dir).unwrap();
        fs::set_permissions(&runtime_dir, fs::Permissions::from_mode(0o700)).unwrap();
        let mount_path = runtime_dir.join("fuse-promise");
        fs::create_dir(&mount_path).unwrap();
        fs::set_permissions(&mount_path, fs::Permissions::from_mode(0o700)).unwrap();
        let _cleanup = RuntimeDirCleanup(runtime_dir.clone());
        let _env = EnvGuard::set("XDG_RUNTIME_DIR", runtime_dir.as_os_str().to_os_string());

        let runtime = Arc::new(Mutex::new(Runtime::new()));
        let state = IpcState::new(runtime);
        state
            .set_mount_status(IpcMountStatus::commit_ready(mount_path.clone()))
            .unwrap();

        thread::spawn(move || serve_state(state).unwrap());
        let socket_path = default_control_socket_path().unwrap();
        wait_for_socket(&socket_path);

        let runtime_dir_c = CString::new(runtime_dir.to_str().unwrap()).unwrap();
        let options = fp_context_options_t {
            struct_size: std::mem::size_of::<fp_context_options_t>() as u32,
            api_version: API_VERSION,
            runtime_dir: runtime_dir_c.as_ptr(),
        };
        let mut context = ptr::null_mut();
        let status = unsafe { fp_context_open(&options, &mut context) };
        assert_eq!(status, FP_OK);
        assert!(!context.is_null());

        let ops = fp_provider_ops_t {
            struct_size: std::mem::size_of::<fp_provider_ops_t>() as u32,
            read: Some(test_read_callback),
        };
        let mut provider = ptr::null_mut();
        let status =
            unsafe { fp_provider_register(context, &ops, std::ptr::null_mut(), &mut provider) };
        assert_eq!(status, FP_OK);
        assert!(!provider.is_null());

        let mut builder = ptr::null_mut();
        let status = unsafe { fp_promise_builder_new(context, provider, &mut builder) };
        assert_eq!(status, FP_OK);
        assert!(!builder.is_null());

        let dir = CString::new("docs").unwrap();
        let dir_node = CString::new("remote-dir-1").unwrap();
        let dir_attr = fp_node_attr_t {
            struct_size: std::mem::size_of::<fp_node_attr_t>() as u32,
            mode: 0o755,
            size: 0,
            mtime_nsec: 0,
        };
        let status =
            unsafe { fp_promise_add_dir(builder, dir.as_ptr(), &dir_attr, dir_node.as_ptr()) };
        assert_eq!(status, FP_OK);

        let file = CString::new("docs/readme.txt").unwrap();
        let file_node = CString::new("remote-file-1").unwrap();
        let file_attr = fp_node_attr_t {
            struct_size: std::mem::size_of::<fp_node_attr_t>() as u32,
            mode: 0o644,
            size: 12,
            mtime_nsec: 0,
        };
        let status =
            unsafe { fp_promise_add_file(builder, file.as_ptr(), &file_attr, file_node.as_ptr()) };
        assert_eq!(status, FP_OK);

        let mut out_path = [0_i8; 512];
        let status = unsafe { fp_promise_commit(builder, out_path.as_mut_ptr(), out_path.len()) };
        assert_eq!(status, FP_OK);

        let visible_path = unsafe { CStr::from_ptr(out_path.as_ptr()) };
        assert_eq!(
            visible_path.to_str().unwrap(),
            mount_path.join("promise-1").to_str().unwrap()
        );
        assert_eq!(
            unsafe { fp_promise_commit(builder, out_path.as_mut_ptr(), out_path.len()) },
            FP_ERR_INVALID_ARGUMENT
        );

        unsafe {
            fp_promise_builder_free(builder);
            fp_provider_unregister(provider);
            fp_context_close(context);
        }
    }

    fn unique_runtime_dir() -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("fuse-promise-ffi-{}-{stamp}", std::process::id()))
    }

    fn wait_for_socket(socket_path: &Path) {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if UnixStream::connect(socket_path).is_ok() {
                return;
            }
            if Instant::now() >= deadline {
                panic!("timed out waiting for {}", socket_path.display());
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    struct EnvGuard {
        key: &'static str,
        old: Option<OsString>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: OsString) -> Self {
            let old = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, old }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.old {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }

    struct RuntimeDirCleanup(PathBuf);

    impl Drop for RuntimeDirCleanup {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn sample_read_request() -> ProviderReadRequest {
        ProviderReadRequest {
            request_id: 42,
            provider_id: 1,
            promise_id: "promise-1".to_owned(),
            relative_path: "docs/readme.txt".to_owned(),
            provider_node_id: "remote-file-1".to_owned(),
            offset: 7,
            length: 8,
        }
    }

    unsafe extern "C" fn test_read_callback(
        request: *const fp_read_request_t,
        response: *mut fp_read_response_t,
        user_data: *mut c_void,
    ) -> fp_status_t {
        if request.is_null() || response.is_null() || !user_data.is_null() {
            return FP_ERR_INVALID_ARGUMENT;
        }

        let request = &*request;
        if CStr::from_ptr(request.promise_id).to_bytes() != b"promise-1"
            || CStr::from_ptr(request.node_id).to_bytes() != b"remote-file-1"
            || CStr::from_ptr(request.relative_path).to_bytes() != b"docs/readme.txt"
            || request.offset != 7
            || request.length != 8
        {
            return FP_ERR_INVALID_ARGUMENT;
        }

        let response = &mut *response;
        let bytes = b"abc";
        if response.buffer_len < bytes.len() || response.buffer.is_null() {
            return FP_ERR_INVALID_ARGUMENT;
        }

        std::ptr::copy_nonoverlapping(bytes.as_ptr(), response.buffer, bytes.len());
        response.bytes_written = bytes.len();
        FP_OK
    }
}
