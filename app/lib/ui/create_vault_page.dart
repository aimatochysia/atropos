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
  bool _lfs = false;
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
        lfs: _lfs,
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
          SegmentedButton<bool>(
            key: const Key('vault-kind'),
            segments: const [
              ButtonSegment(
                value: false,
                icon: Icon(Icons.lock_outline),
                label: Text('Vault'),
              ),
              ButtonSegment(
                value: true,
                icon: Icon(Icons.folder_outlined),
                label: Text('Vault LFS'),
              ),
            ],
            selected: {_lfs},
            onSelectionChanged: _busy
                ? null
                : (v) => setState(() => _lfs = v.first),
          ),
          const SizedBox(height: 8),
          Text(
            _lfs
                ? 'A folder that encrypts each file on its own, so very large '
                      'files stay fast to add and change. Good for big photo '
                      'or video collections, and for syncing with cloud '
                      'drives or git.'
                : 'One encrypted .atropos file, easy to move, upload or send. '
                      'Every change rewrites the whole file, so it suits '
                      'documents and smaller collections. Pick Vault LFS for '
                      'many gigabytes.',
            key: const Key('vault-kind-help'),
            style: t.bodySmall,
          ),
          const SizedBox(height: 4),
          Text(
            'Both kinds open with a password, a recovery phrase, fingerprint '
            'unlock and security cards. You can make a copy as the other '
            'kind later.',
            style: t.bodySmall,
          ),
          const SizedBox(height: 12),
          TextField(
            key: const Key('vault-name'),
            controller: _name,
            decoration: InputDecoration(
              labelText: 'Vault name',
              helperText: _lfs
                  ? 'Becomes the folder name. Visible to anyone who sees '
                        'the folder.'
                  : 'Becomes the file name (with .atropos). Visible to '
                        'anyone who sees the file.',
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
