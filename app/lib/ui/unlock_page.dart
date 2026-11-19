import 'package:flutter/material.dart';

import '../core/errors.dart';
import '../src/rust/api/vault.dart' as core;
import '../state/app_controller.dart';
import 'secret_dialog.dart';
import 'secret_field.dart';
import 'vault_page.dart';
import 'widgets/feedback.dart';

class UnlockPage extends StatefulWidget {
  const UnlockPage({super.key, required this.path, required this.name});

  final String path;
  final String name;

  @override
  State<UnlockPage> createState() => _UnlockPageState();
}

class _UnlockPageState extends State<UnlockPage> {
  final _secret = SecretInput();
  bool _recovery = false;
  int _attempt = 0;
  bool _busy = false;
  String? _error;

  @override
  void dispose() {
    _secret.clear();
    super.dispose();
  }

  Future<void> _unlock() async {
    if (_busy || _secret.isEmpty()) return;
    if (_recovery && !core.recoveryPhraseIsValid(phrase: _secret)) {
      setState(
        () => _error =
            'That is not a valid recovery phrase. Check every word and '
            'their order.',
      );
      return;
    }
    final app = AppScope.read(context);
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      final session = _recovery
          ? await core.unlockWithRecoveryPhrase(
              path: widget.path,
              phrase: _secret,
            )
          : await core.unlockWithPassword(path: widget.path, password: _secret);
      _secret.clear();
      await app.opened(session);
      if (!mounted) return;
      final usedRecovery = _recovery;
      final nav = Navigator.of(context);
      nav.pushReplacement(
        MaterialPageRoute<void>(builder: (_) => VaultPage(session: session)),
      );
      if (usedRecovery) _offerNewPassword(nav.context, session);
    } catch (e) {
      if (mounted) {
        setState(() {
          _error = describeError(e);
          _secret.clear();
          _attempt++;
        });
      }
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  /// After a recovery unlock the person has most likely forgotten the
  /// password, so offer to set a new one straight away.
  Future<void> _offerNewPassword(
    BuildContext context,
    core.VaultSession session,
  ) async {
    await Future<void>.delayed(const Duration(milliseconds: 400));
    if (!context.mounted) return;
    final pw = await askSecret(
      context,
      title: 'Set a new password',
      message:
          'You unlocked with the recovery phrase. Choose a new password '
          'for everyday use; the recovery phrase keeps working.',
      label: 'New password',
      action: 'Save',
      isNew: true,
    );
    if (pw == null || !context.mounted) return;
    await guarded(
      context,
      () => session.changePassword(newPassword: pw),
      success: (_) => 'Password changed.',
    );
    pw.clear();
  }

  @override
  Widget build(BuildContext context) {
    final secure = AppScope.of(context).settings.secureKeyboard;
    final scheme = Theme.of(context).colorScheme;
    return Scaffold(
      appBar: AppBar(title: Text(widget.name)),
      body: ListView(
        padding: const EdgeInsets.all(16),
        children: [
          Icon(Icons.lock_outline, size: 48, color: scheme.primary),
          const SizedBox(height: 8),
          Text(
            widget.path,
            textAlign: TextAlign.center,
            style: Theme.of(context).textTheme.bodySmall,
          ),
          const SizedBox(height: 16),
          SecretField(
            key: ValueKey('unlock-$_recovery-$_attempt'),
            input: _secret,
            label: _recovery ? 'Recovery phrase (24 words)' : 'Password',
            secureKeyboard: secure,
            lettersOnly: _recovery,
            autofocus: true,
            errorText: _error,
            helper: _recovery ? 'Lowercase words separated by spaces.' : null,
            onChanged: () => setState(() => _error = null),
            onSubmitted: _unlock,
          ),
          const SizedBox(height: 16),
          FilledButton.icon(
            key: const Key('unlock'),
            onPressed: _busy ? null : _unlock,
            icon: _busy
                ? const SizedBox.square(
                    dimension: 16,
                    child: CircularProgressIndicator(strokeWidth: 2),
                  )
                : const Icon(Icons.lock_open),
            label: Text(_busy ? 'Unlocking…' : 'Unlock'),
          ),
          const SizedBox(height: 8),
          TextButton(
            key: const Key('toggle-recovery'),
            onPressed: _busy
                ? null
                : () => setState(() {
                    _recovery = !_recovery;
                    _secret.clear();
                    _error = null;
                  }),
            child: Text(
              _recovery ? 'Use the password instead' : 'Forgot the password?',
            ),
          ),
        ],
      ),
    );
  }
}
