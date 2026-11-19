# Atropos

Encrypt folders and files as securely as possible while staying convenient, so the
encrypted result can travel through cloud drives, USB sticks, chat uploads and git
repositories, and be opened on Android, Linux, Windows or macOS.

All cryptography and file logic lives in the Rust core (`app/rust`); Flutter only
draws the screens.

| Role | Algorithm | Standard |
| --- | --- | --- |
| Post-quantum key wrap | ML-KEM-1024 | FIPS 203 |
| Classical key wrap | ECDH P-384 | SP 800-56A r3 |
| Hybrid combiner | HKDF-SHA-384 | SP 800-56C r2 |
| File encryption | AES-256-GCM, 64 KiB chunks (STREAM) | FIPS 197, SP 800-38D |
| Password | Argon2id, 256 MiB | RFC 9106 |

The algorithms are FIPS-approved (apart from Argon2id); the app is not a FIPS 140-3
validated module.

## Status

v0.0: project scaffold, release pipeline, signed in-app updates and an on-device
crypto self-test. Vaults arrive in v0.1. See the plan for the roadmap.

## Building

```sh
cd app
flutter pub get
flutter_rust_bridge_codegen generate   # after changing app/rust/src/api
flutter run
```

Rust tests: `cd app/rust && cargo test`.

Releases are built by the **Release** workflow (Actions tab, run on `main`). It needs
these repository secrets: `ANDROID_KEYSTORE_BASE64`, `ANDROID_KEYSTORE_PASSWORD`,
`ANDROID_KEY_ALIAS`, `ANDROID_KEY_PASSWORD`, `UPDATE_SIGNING_KEY`,
`UPDATE_SIGNING_PASSWORD`.
