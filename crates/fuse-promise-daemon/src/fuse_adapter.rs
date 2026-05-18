use fuse_promise_ipc::IpcMountStatus;
use fuse_promise_runtime::Runtime;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[cfg(feature = "fuse-mount")]
pub struct FuseMount {
    _session: fuser::BackgroundSession,
}

#[cfg(not(feature = "fuse-mount"))]
pub struct FuseMount;

#[cfg(feature = "fuse-mount")]
struct PromiseFilesystem {
    _runtime: Arc<Mutex<Runtime>>,
}

#[cfg(feature = "fuse-mount")]
impl fuser::Filesystem for PromiseFilesystem {}

#[cfg(feature = "fuse-mount")]
pub fn start(mount_path: &Path, runtime: Arc<Mutex<Runtime>>) -> io::Result<Option<FuseMount>> {
    std::fs::create_dir_all(mount_path)?;

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

    let filesystem = PromiseFilesystem { _runtime: runtime };
    let session = fuser::spawn_mount2(filesystem, mount_path, &config)?;
    Ok(Some(FuseMount { _session: session }))
}

#[cfg(not(feature = "fuse-mount"))]
pub fn start(_mount_path: &Path, _runtime: Arc<Mutex<Runtime>>) -> io::Result<Option<FuseMount>> {
    Ok(None)
}

pub fn mount_status(mount_path: &Path, mount: &Option<FuseMount>) -> IpcMountStatus {
    if mount.is_some() {
        IpcMountStatus::mounted(mount_path.to_path_buf())
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
