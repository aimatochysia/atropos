import 'dart:io';

import 'package:flutter/services.dart';

import '../src/rust/api/card.dart' as core;

/// An open line to one smartcard. [transmit] sends one APDU and returns the
/// answer; an empty answer means the card is gone.
class CardLink {
  CardLink({required this.name, required this.transmit, required this.close});

  final String name;
  final Future<Uint8List> Function(Uint8List apdu) transmit;
  final Future<void> Function() close;
}

/// How Atropos reaches cards on this platform: NFC on Android, PC/SC
/// readers on desktops.
abstract interface class CardAccess {
  /// True when cards are held to the device (NFC) rather than inserted.
  bool get contactless;

  /// Null when ready, otherwise why cards can't be used right now.
  Future<String?> problem();

  /// Waits for a card (NFC) or opens the first reader with a card. Null when
  /// the person cancelled.
  Future<CardLink?> connect();

  /// Stops [connect] waiting.
  Future<void> cancel();
}

class PlatformCardAccess implements CardAccess {
  const PlatformCardAccess();

  static const _nfc = MethodChannel('atropos/nfc');

  @override
  bool get contactless => Platform.isAndroid;

  @override
  Future<String?> problem() async {
    if (Platform.isAndroid) {
      final s = await _nfc.invokeMethod<String>('status');
      return switch (s) {
        'ready' => null,
        'off' => 'NFC is off. Turn it on in the system settings.',
        _ => 'This device has no NFC.',
      };
    }
    try {
      final readers = await core.cardReaders();
      return readers.isEmpty ? 'No smartcard reader is connected.' : null;
    } catch (e) {
      return e.toString();
    }
  }

  @override
  Future<CardLink?> connect() async {
    if (Platform.isAndroid) {
      final ok = await _nfc.invokeMethod<bool>('waitForCard') ?? false;
      if (!ok) return null;
      return CardLink(
        name: 'NFC card',
        transmit: (apdu) async =>
            await _nfc.invokeMethod<Uint8List>('transceive', {'apdu': apdu}) ??
            Uint8List(0),
        close: () => _nfc.invokeMethod<void>('close'),
      );
    }
    final reader = await core.openCardReader();
    return CardLink(
      name: reader.name,
      transmit: (apdu) => reader.transmit(apdu: apdu),
      close: () async => reader.dispose(),
    );
  }

  @override
  Future<void> cancel() async {
    if (Platform.isAndroid) await _nfc.invokeMethod<void>('cancel');
  }
}
