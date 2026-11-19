import 'dart:io';

import 'package:flutter/services.dart';

enum BiometricStatus { ready, noneEnrolled, unavailable }

/// A failure the person should read, such as "fingerprints changed".
class BiometricError implements Exception {
  BiometricError(this.message, {this.invalidated = false});
  final String message;

  /// The keystore key is gone (new fingerprint or face enrolled).
  final bool invalidated;

  @override
  String toString() => message;
}

/// The platform keystore that guards each vault's biometric secret. The
/// secret is made by the Rust core; this only asks the hardware to protect
/// it behind a fingerprint or face check, and to give it back after one.
abstract interface class Biometrics {
  Future<BiometricStatus> status();
  Future<bool> has(String vaultId);

  /// Prompts, then stores [secret]. False when the person cancelled.
  /// [secret] is zeroed afterwards.
  Future<bool> enroll(String vaultId, Uint8List secret);

  /// Prompts and returns the secret, or null when cancelled.
  Future<Uint8List?> unlock(String vaultId, {String? title});
  Future<void> remove(String vaultId);
}

class PlatformBiometrics implements Biometrics {
  const PlatformBiometrics();

  static const _ch = MethodChannel('atropos/biometric');

  bool get _supported => Platform.isAndroid;

  @override
  Future<BiometricStatus> status() async {
    if (!_supported) return BiometricStatus.unavailable;
    final s = await _ch.invokeMethod<String>('status');
    return switch (s) {
      'ready' => BiometricStatus.ready,
      'none_enrolled' => BiometricStatus.noneEnrolled,
      _ => BiometricStatus.unavailable,
    };
  }

  @override
  Future<bool> has(String vaultId) async {
    if (!_supported) return false;
    return await _ch.invokeMethod<bool>('has', {'vaultId': vaultId}) ?? false;
  }

  @override
  Future<bool> enroll(String vaultId, Uint8List secret) async {
    try {
      final ok = await _ch.invokeMethod<bool>('enroll', {
        'vaultId': vaultId,
        'secret': secret,
        'title': 'Turn on fingerprint unlock',
      });
      return ok ?? false;
    } on PlatformException catch (e) {
      throw BiometricError(e.message ?? 'Fingerprint unlock failed.');
    } finally {
      secret.fillRange(0, secret.length, 0);
    }
  }

  @override
  Future<Uint8List?> unlock(String vaultId, {String? title}) async {
    try {
      return await _ch.invokeMethod<Uint8List>('unlock', {
        'vaultId': vaultId,
        'title': ?title,
      });
    } on PlatformException catch (e) {
      throw BiometricError(
        e.message ?? 'Fingerprint unlock failed.',
        invalidated: e.code == 'invalidated' || e.code == 'missing',
      );
    }
  }

  @override
  Future<void> remove(String vaultId) async {
    if (!_supported) return;
    await _ch.invokeMethod<void>('remove', {'vaultId': vaultId});
  }
}
