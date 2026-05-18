#![allow(non_camel_case_types)]

use fuse_promise_runtime::{
    default_mount_path, validate_runtime_dir_path, NodeAttr, PromiseBuilder, ProviderId, Runtime,
    Status, API_VERSION,
};
use std::ffi::CStr;
use std::os::raw::{c_char, c_void};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::ptr;
use std::sync::{Arc, Mutex};

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
    _read: fp_provider_read_fn,
    _user_data: *mut c_void,
}

pub struct fp_promise_builder {
    inner: Arc<ContextInner>,
    provider_id: ProviderId,
    builder: Mutex<Option<PromiseBuilder>>,
}

pub enum fp_materialize_job {}

struct ContextInner {
    runtime: Mutex<Runtime>,
    _runtime_dir: PathBuf,
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

        let runtime_dir = runtime_dir_from_options(options)?;
        let context = fp_context {
            inner: Arc::new(ContextInner {
                runtime: Mutex::new(Runtime::new()),
                _runtime_dir: runtime_dir,
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
        let mut runtime = lock_runtime(&inner)?;
        let id = runtime.register_provider();
        drop(runtime);

        let provider = fp_provider {
            inner,
            id,
            _read: read,
            _user_data: user_data,
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

    let provider = Box::from_raw(provider);
    {
        let inner = provider.inner.clone();
        if let Ok(mut runtime) = inner.runtime.lock() {
            let _ = runtime.unregister_provider(provider.id);
        };
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

        let inner = (*context).inner.clone();
        let provider_id = (*provider).id;
        if !lock_runtime(&inner)?.has_provider(provider_id) {
            return Err(FP_ERR_PROVIDER_GONE);
        }

        let builder = fp_promise_builder {
            inner,
            provider_id,
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

        if builder.builder.lock().map_err(|_| FP_ERR_IO)?.is_none() {
            return Err(FP_ERR_INVALID_ARGUMENT);
        }

        if !lock_runtime(&builder.inner)?.has_provider(builder.provider_id) {
            return Err(FP_ERR_PROVIDER_GONE);
        }

        Ok(FP_ERR_UNAVAILABLE)
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
        let _promise_path = cstr_to_str(promise_path)?;
        let _target_dir = cstr_to_str(target_dir)?;
        materialize_options(options)?;

        Ok(FP_ERR_UNAVAILABLE)
    })
}

fn ffi_guard(action: impl FnOnce() -> Result<fp_status_t, fp_status_t>) -> fp_status_t {
    match catch_unwind(AssertUnwindSafe(action)) {
        Ok(Ok(status)) => status,
        Ok(Err(status)) => status,
        Err(_) => FP_ERR_IO,
    }
}

unsafe fn runtime_dir_from_options(
    options: *const fp_context_options_t,
) -> Result<PathBuf, fp_status_t> {
    if options.is_null() {
        return default_mount_path().map_err(status_to_ffi);
    }

    if ((*options).struct_size as usize) < required_context_options_size() {
        return Err(FP_ERR_INVALID_ARGUMENT);
    }
    if (*options).api_version != API_VERSION {
        return Err(FP_ERR_VERSION_MISMATCH);
    }

    let runtime_dir = (*options).runtime_dir;
    if runtime_dir.is_null() {
        return default_mount_path().map_err(status_to_ffi);
    }

    let runtime_dir = PathBuf::from(cstr_to_str(runtime_dir)?);
    validate_runtime_dir_path(&runtime_dir).map_err(status_to_ffi)?;
    Ok(runtime_dir.join("fuse-promise"))
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

unsafe fn materialize_options(options: *const fp_materialize_options_t) -> Result<(), fp_status_t> {
    if options.is_null() {
        return Ok(());
    }
    if ((*options).struct_size as usize) < required_materialize_options_size() {
        return Err(FP_ERR_INVALID_ARGUMENT);
    }
    match (*options).conflict_policy {
        FP_CONFLICT_FAIL | FP_CONFLICT_OVERWRITE | FP_CONFLICT_RENAME => Ok(()),
        _ => Err(FP_ERR_INVALID_ARGUMENT),
    }
}

unsafe fn builder_mut<'a>(
    builder: *mut fp_promise_builder,
) -> Result<&'a fp_promise_builder, fp_status_t> {
    if builder.is_null() {
        return Err(FP_ERR_INVALID_ARGUMENT);
    }

    Ok(&*builder)
}

fn lock_runtime(inner: &ContextInner) -> Result<std::sync::MutexGuard<'_, Runtime>, fp_status_t> {
    inner.runtime.lock().map_err(|_| FP_ERR_IO)
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
