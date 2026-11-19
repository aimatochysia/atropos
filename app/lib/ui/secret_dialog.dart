import 'package:flutter/material.dart';

import '../state/app_controller.dart';
import 'secret_field.dart';

/// Asks for a password or passphrase. With [isNew] the person types it twice
/// and it must pass the strength check. Returns null when cancelled; the
/// caller owns the returned input and should [SecretInput.clear] it.
Future<SecretInput?> askSecret(
  BuildContext context, {
  required String title,
  String? message,
  String label = 'Passphrase',
  String action = 'OK',
  bool isNew = false,
}) {
  final secure = AppScope.read(context).settings.secureKeyboard;
  return showDialog<SecretInput>(
    context: context,
    builder: (_) => _SecretDialog(
      title: title,
      message: message,
      label: label,
      action: action,
      isNew: isNew,
      secure: secure,
    ),
  );
}

class _SecretDialog extends StatefulWidget {
  const _SecretDialog({
    required this.title,
    required this.message,
    required this.label,
    required this.action,
    required this.isNew,
    required this.secure,
  });

  final String title;
  final String? message;
  final String label;
  final String action;
  final bool isNew;
  final bool secure;

  @override
  State<_SecretDialog> createState() => _SecretDialogState();
}

class _SecretDialogState extends State<_SecretDialog> {
  final _first = SecretInput();
  final _second = SecretInput();
  Strength? _strength;
  String? _mismatch;
  bool _returned = false;

  @override
  void dispose() {
    if (!_returned) _first.clear();
    _second.clear();
    super.dispose();
  }

  bool get _ok {
    if (_first.isEmpty()) return false;
    if (!widget.isNew) return true;
    return (_strength?.acceptable ?? false) && !_second.isEmpty();
  }

  void _submit() {
    if (!_ok) return;
    if (widget.isNew && !_first.matches(other: _second)) {
      setState(() => _mismatch = 'The two entries are different.');
      return;
    }
    _returned = true;
    Navigator.pop(context, _first);
  }

  @override
  Widget build(BuildContext context) {
    return AlertDialog(
      title: Text(widget.title),
      scrollable: true,
      content: SizedBox(
        width: 460,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            if (widget.message != null) ...[
              Text(widget.message!),
              const SizedBox(height: 12),
            ],
            SecretField(
              key: const Key('secret-first'),
              input: _first,
              label: widget.label,
              secureKeyboard: widget.secure,
              autofocus: true,
              onChanged: () => setState(() {
                _mismatch = null;
                if (widget.isNew) _strength = _first.strength();
              }),
              onSubmitted: widget.isNew ? null : _submit,
            ),
            if (widget.isNew) ...[
              StrengthMeter(strength: _first.isEmpty() ? null : _strength),
              const SizedBox(height: 12),
              SecretField(
                key: const Key('secret-second'),
                input: _second,
                label: 'Type it again',
                secureKeyboard: widget.secure,
                errorText: _mismatch,
                onChanged: () => setState(() => _mismatch = null),
                onSubmitted: _submit,
              ),
            ],
          ],
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.pop(context),
          child: const Text('Cancel'),
        ),
        FilledButton(
          key: const Key('secret-ok'),
          onPressed: _ok ? _submit : null,
          child: Text(widget.action),
        ),
      ],
    );
  }
}
