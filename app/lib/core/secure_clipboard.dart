import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

/// Copies text out of Atropos and takes it back off the clipboard after a
/// minute, or when the app locks, unless something else was copied since.
class SecureClipboard {
  SecureClipboard._();

  static const keep = Duration(seconds: 60);
  static Timer? _timer;
  static String? _ours;

  static Future<void> copy(String text) async {
    await Clipboard.setData(ClipboardData(text: text));
    _remember(text);
  }

  /// Call right after something else (a text field's menu) copied.
  static Future<void> afterCopy() async {
    try {
      final d = await Clipboard.getData(Clipboard.kTextPlain);
      if (d?.text != null) _remember(d!.text!);
    } catch (_) {}
  }

  static void _remember(String text) {
    _ours = text;
    _timer?.cancel();
    _timer = Timer(keep, clearIfOurs);
  }

  /// Clears the clipboard if it still holds what Atropos put there.
  static Future<void> clearIfOurs() async {
    _timer?.cancel();
    _timer = null;
    final ours = _ours;
    _ours = null;
    if (ours == null) return;
    try {
      final d = await Clipboard.getData(Clipboard.kTextPlain);
      if (d?.text == ours) {
        await Clipboard.setData(const ClipboardData(text: ''));
      }
    } catch (_) {}
  }

  static const notice =
      'Copied. Other apps can read the clipboard, so Atropos clears it '
      'after 60 seconds or when it locks.';

  /// Wraps a text field's or selection's menu so Copy and Cut are cleared
  /// later too.
  static List<ContextMenuButtonItem> guard(
    BuildContext context,
    List<ContextMenuButtonItem> items,
  ) => [
    for (final i in items)
      if (i.type == ContextMenuButtonType.copy ||
          i.type == ContextMenuButtonType.cut)
        i.copyWith(
          onPressed: () {
            i.onPressed?.call();
            Future<void>.delayed(const Duration(milliseconds: 100), afterCopy);
            final m = ScaffoldMessenger.maybeOf(context);
            m?.hideCurrentSnackBar();
            m?.showSnackBar(
              const SnackBar(
                content: Text(notice),
                behavior: SnackBarBehavior.floating,
              ),
            );
          },
        )
      else
        i,
  ];
}
