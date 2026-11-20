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

- v0.1: vaults with a password and a 24-word recovery phrase, import, browse,
  export behind a confirmation, sealed packages, auto-lock.
- v0.2: fingerprint or face unlock on Android, renewed by a password or card
  unlock at least every 24 hours.
- v0.3: security cards (OpenPGP and PIV) over NFC on Android and PC/SC readers
  on desktops.
- v0.4: in-app viewer for pictures, PDFs, text and Markdown. Rust decrypts,
  decodes and renders (PDF with the pure-Rust `hayro` renderer); Flutter only
  receives pixels or text lines, and the plaintext is wiped when the viewer
  closes or the vault locks. Other file types show their details and can be
  exported.
- v0.5: "Check and repair" decrypts and verifies every file, finds damaged or
  missing files, leftovers from interrupted writes and unreadable parts of the
  file list (which no longer stop the vault from opening), and fixes them.
  Password protection strength can be set to 64 MB, 256 MB or 1 GB.
- v0.5.1: "Re-key vault" re-encrypts everything under all-new keys and a new
  recovery phrase (password unchanged), for when a key or phrase may have
  leaked. Earlier copies still open with the old credentials.

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
