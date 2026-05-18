fn main() {
    println!("cargo:rerun-if-env-changed=FUSE_PROMISE_SONAME_MAJOR");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        let soname_major =
            std::env::var("FUSE_PROMISE_SONAME_MAJOR").unwrap_or_else(|_| "0".to_owned());
        if soname_major.is_empty() || !soname_major.bytes().all(|byte| byte.is_ascii_digit()) {
            panic!("FUSE_PROMISE_SONAME_MAJOR must be a non-negative integer");
        }
        println!("cargo:rustc-link-arg-cdylib=-Wl,-soname,libfusepromise.so.{soname_major}");
    }
}
