import '../src/rust/api/app.dart' as app;
import '../src/rust/api/selftest.dart' as st;

export '../src/rust/api/selftest.dart' show SelfTestReport, SelfTestStep;

/// What the UI needs from the Rust core's cryptography. All crypto runs in
/// Rust; Dart only receives results, never keys.
abstract interface class CryptoService {
  String get coreVersion;
  String get backend;
  Future<st.SelfTestReport> runSelfTest();
}

class RustCryptoService implements CryptoService {
  const RustCryptoService();

  @override
  String get coreVersion => app.coreVersion();

  @override
  String get backend => app.cryptoBackend();

  @override
  Future<st.SelfTestReport> runSelfTest() => st.runCryptoSelfTest();
}
