/// Version of the Rust core, taken from Cargo.toml at build time.
#[flutter_rust_bridge::frb(sync)]
pub fn core_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// The library that performs the cryptography.
#[flutter_rust_bridge::frb(sync)]
pub fn crypto_backend() -> String {
    "AWS-LC (aws-lc-rs) + RustCrypto Argon2".to_string()
}

#[flutter_rust_bridge::frb(init)]
pub fn init_app() {
    flutter_rust_bridge::setup_default_user_utils();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_not_empty() {
        assert!(!core_version().is_empty());
    }
}
