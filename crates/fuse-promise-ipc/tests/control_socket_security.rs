use fuse_promise_ipc::query_status;
use std::fs;
use std::io;

#[test]
fn client_rejects_non_socket_control_path() {
    let temp_dir = tempfile::tempdir().unwrap();
    let socket_path = temp_dir.path().join("fuse-promise.sock");
    fs::write(&socket_path, b"not a socket").unwrap();

    let error = query_status(&socket_path).unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert_eq!(error.to_string(), "control socket path is not a socket");
}
