import 'package:flutter/services.dart';

import '../src/rust/error.dart';

/// Turns any error from the core into one sentence a person can act on.
String describeError(Object error) {
  if (error is CoreError) {
    return switch (error) {
      CoreError_Network(:final message) =>
        'Could not reach the server. Check your connection. ($message)',
      CoreError_GitHub(:final status, :final message) =>
        'GitHub returned $status: ${_capitalize(message)}',
      CoreError_Integrity(:final message) =>
        'Update rejected: $message. Nothing was installed.',
      CoreError_InvalidInput(:final message) => _capitalize(message),
      CoreError_Io(:final message) => _capitalize(message),
      CoreError_DecryptFailed() =>
        'Could not decrypt: the key is wrong or the data was changed.',
      CoreError_Crypto() => 'A cryptographic operation failed.',
    };
  }
  if (error is PlatformException) {
    return error.message ?? 'Something went wrong (${error.code}).';
  }
  return error.toString();
}

String _capitalize(String s) =>
    s.isEmpty ? s : '${s[0].toUpperCase()}${s.substring(1)}';
