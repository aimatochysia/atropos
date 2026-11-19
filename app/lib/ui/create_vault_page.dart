import 'package:flutter/material.dart';

import '../core/errors.dart';
import '../src/rust/api/vault.dart' as core;
import '../state/app_controller.dart';
import 'recovery_page.dart';
import 'secret_field.dart';
import 'storage_access.dart';
import 'vault_page.dart';

class CreateVaultPage extends StatefulWidget {
  const CreateVaultPage({super.key});

  @override
  State<CreateVaultPage> createState() => _CreateVaultPageState();
}

class _CreateVaultPageState extends State<CreateVaultPage> {
  final _name = TextEditingController(text: 'My vault');
  final _password = SecretInput();
  final _again = SecretInput();
  String? _location;
  Strength? _strength;
  String? _mismatch;
  String? _error;
  bool _busy = false;

  @override
  void initState() {
    super.initState();
    AppScope.read(context).files.defaultVaultParent().then((p) {
      if (mounted) setState(() => _location ??= p);
    });
  }

  @override
  void dispose() {
    _password.clear();
    _again.clear();
    _name.dispose();
    super.dispose();
  }

  bool get _ready =>
      !_busy &&
      _name.text.trim().isNotEmpty &&
      _location != null &&
      (_strength?.acceptable ?? false) &&
      !_again.isEmpty();

  Future<void> _chooseLocation() async {
    final app = AppScope.read(context);
    if (!await ensureStorageAccess(context)) return;
    final dir = await app.whilePicking(
      () => app.files.pickFolder(title: 'Create vault here'),
    );
    if (dir != null && mounted) setState(() => _location = dir);
  }

  Future<void> _create() async {
    if (!_password.matches(other: _again)) {
      setState(() => _mismatch = 'The two passwords are different.');
      return;
    }
    if (!await ensureStorageAccess(context) || !mounted) return;
    final app = AppScope.read(context);
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      final created = await core.createVault(
        parentDir: _location!,
        name: _name.text.trim(),
        password: _password,
      );
      _password.clear();
      _again.clear();
      final session = created.session;
      final phrase = created.recoveryPhrase;
      await app.opened(session);
      if (!mounted) return;
      final nav = Navigator.of(context);
      await nav.pushReplacement(
        MaterialPageRoute<void>(
          builder: (_) => RecoveryPhrasePage(
            phrase: phrase,
            onDone: (ctx) => Navigator.of(ctx).pushReplacement(
              MaterialPageRoute<void>(
                builder: (_) => VaultPage(session: session),
              ),
            ),
          ),
        ),
      );
    } catch (e) {
      if (mounted) setState(() => _error = describeError(e));
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final secure = AppScope.of(context).settings.secureKeyboard;
    final t = Theme.of(context).textTheme;
    return Scaffold(
      appBar: AppBar(title: const Text('New vault')),
      body: ListView(
        padding: const EdgeInsets.all(16),
        children: [
          TextField(
            key: const Key('vault-name'),
            controller: _name,
            decoration: const InputDecoration(
              labelText: 'Vault name',
              helperText:
                  'Becomes the folder name. Visible to anyone who '
                  'sees the folder.',
            ),
            onChanged: (_) => setState(() {}),
          ),
          const SizedBox(height: 12),
          ListTile(
            contentPadding: EdgeInsets.zero,
            leading: const Icon(Icons.folder_outlined),
            title: const Text('Location'),
            subtitle: Text(_location ?? '…'),
            trailing: TextButton(
              onPressed: _busy ? null : _chooseLocation,
              child: const Text('Change'),
            ),
          ),
          const SizedBox(height: 12),
          SecretField(
            key: const Key('create-password'),
            input: _password,
            label: 'Password',
            secureKeyboard: secure,
            helper: 'A few unrelated words are strong and easy to remember.',
            onChanged: () => setState(() {
              _mismatch = null;
              _strength = _password.isEmpty() ? null : _password.strength();
            }),
          ),
          StrengthMeter(strength: _strength),
          const SizedBox(height: 12),
          SecretField(
            key: const Key('create-password-again'),
            input: _again,
            label: 'Type the password again',
            secureKeyboard: secure,
            errorText: _mismatch,
            onChanged: () => setState(() => _mismatch = null),
            onSubmitted: _ready ? _create : null,
          ),
          const SizedBox(height: 16),
          Text(
            'Next you get a 24-word recovery phrase. It is the only way back '
            'in if you forget the password.',
            style: t.bodySmall,
          ),
          if (_error != null) ...[
            const SizedBox(height: 12),
            Text(
              _error!,
              style: TextStyle(color: Theme.of(context).colorScheme.error),
            ),
          ],
          const SizedBox(height: 16),
          FilledButton.icon(
            key: const Key('create-vault'),
            onPressed: _ready ? _create : null,
            icon: _busy
                ? const SizedBox.square(
                    dimension: 16,
                    child: CircularProgressIndicator(strokeWidth: 2),
                  )
                : const Icon(Icons.lock_outline),
            label: Text(_busy ? 'Creating…' : 'Create vault'),
          ),
        ],
      ),
    );
  }
}
