import 'dart:io';

import 'package:flutter/material.dart';

import '../core/crypto_service.dart';
import '../core/format.dart';
import '../src/rust/api/vault.dart' as core;
import '../state/app_controller.dart';
import 'check_page.dart' show KeysProgressDialog;
import 'create_vault_page.dart';
import 'package_browser_page.dart';
import 'recovery_page.dart';
import 'secret_dialog.dart';
import 'settings_page.dart';
import 'storage_access.dart';
import 'unlock_page.dart';
import 'update_ui.dart';
import 'vault_page.dart';
import 'widgets/feedback.dart';

/// Lists known vaults and starts everything else.
class HomePage extends StatelessWidget {
  const HomePage({super.key, required this.crypto, this.appVersion = ''});

  final CryptoService crypto;
  final String appVersion;

  Future<void> _addExisting(BuildContext context) async {
    final app = AppScope.read(context);
    final kind = await showModalBottomSheet<String>(
      context: context,
      builder: (ctx) => SafeArea(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            ListTile(
              key: const Key('open-vault-file'),
              leading: const Icon(Icons.lock_outline),
              title: const Text('A vault file (.atropos)'),
              onTap: () => Navigator.pop(ctx, 'file'),
            ),
            ListTile(
              key: const Key('open-vault-folder'),
              leading: const Icon(Icons.folder_outlined),
              title: const Text('A Vault LFS folder'),
              onTap: () => Navigator.pop(ctx, 'folder'),
            ),
          ],
        ),
      ),
    );
    if (kind == null || !context.mounted) return;
    if (!await ensureStorageAccess(context) || !context.mounted) return;
    if (kind == 'file') {
      final picked = await app.whilePicking(app.files.pickFileInPlace);
      if (picked == null || !context.mounted) return;
      if (picked.temporary) {
        await picked.cleanUp();
        if (!context.mounted) return;
        // A copy would leave changes behind in the cache: open in place only.
        await browsePackageFlow(context);
        return;
      }
      await openAtroposFile(context, picked.paths.first);
      return;
    }
    final dir = await app.whilePicking(
      () => app.files.pickFolder(title: 'Open vault'),
    );
    if (dir == null || !context.mounted) return;
    if (!core.isVault(path: dir)) {
      showMessage(
        context,
        'That folder is not an Atropos vault. Pick the folder that contains '
        'vault.hdr.',
        error: true,
      );
      return;
    }
    await app.store.remember(dir);
    await app.refreshVaults();
    if (!context.mounted) return;
    _openVault(context, dir, _name(dir));
  }

  static String _name(String path) {
    final n =
        path.split(RegExp(r'[\\/]')).where((p) => p.isNotEmpty).lastOrNull ??
        path;
    return n.toLowerCase().endsWith('.atropos')
        ? n.substring(0, n.length - 8)
        : n;
  }

  static void _openVault(BuildContext context, String path, String name) {
    final app = AppScope.read(context);
    final s = app.session(path);
    Navigator.of(context).push(
      MaterialPageRoute<void>(
        builder: (_) => s != null
            ? VaultPage(session: s)
            : UnlockPage(path: path, name: name),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final app = AppScope.of(context);
    return Scaffold(
      appBar: AppBar(
        title: const Text('Atropos'),
        actions: [
          if (app.anyOpen)
            IconButton(
              key: const Key('lock-all'),
              tooltip: 'Lock all vaults',
              icon: const Icon(Icons.lock_outline),
              onPressed: app.lockAll,
            ),
          IconButton(
            tooltip: 'Settings',
            icon: const Icon(Icons.settings_outlined),
            onPressed: () => Navigator.of(context).push(
              MaterialPageRoute<void>(
                builder: (_) =>
                    SettingsPage(crypto: crypto, appVersion: appVersion),
              ),
            ),
          ),
        ],
      ),
      floatingActionButton: FloatingActionButton.extended(
        key: const Key('new-vault'),
        onPressed: () => Navigator.of(context).push(
          MaterialPageRoute<void>(builder: (_) => const CreateVaultPage()),
        ),
        icon: const Icon(Icons.add),
        label: const Text('New vault'),
      ),
      body: !app.loaded
          ? const Center(child: CircularProgressIndicator())
          : ListView(
              padding: const EdgeInsets.fromLTRB(16, 8, 16, 96),
              children: [
                const UpdateBanner(),
                if (app.vaults.isEmpty) const _Intro(),
                if (app.vaults.isNotEmpty) const _Header('Vaults'),
                for (final v in app.vaults)
                  _VaultTile(
                    vault: v,
                    open: app.isOpen(v.path),
                    onTap: () => _openVault(context, v.path, v.name),
                  ),
                TextButton.icon(
                  key: const Key('add-existing'),
                  onPressed: () => _addExisting(context),
                  icon: const Icon(Icons.folder_open_outlined),
                  label: const Text('Open an existing vault'),
                ),
                if (app.packages.isNotEmpty) ...[
                  const _Header('Older .atropos files'),
                  Card(
                    child: Column(
                      children: [
                        for (final p in app.packages)
                          ListTile(
                            key: Key('recent-${p.name}'),
                            leading: const Icon(Icons.upgrade),
                            title: Text(p.name),
                            subtitle: Text(
                              [
                                'Older format: tap to upgrade it to a vault',
                                if (p.lastOpened > 0)
                                  'opened ${relativeTime(p.lastOpened)}',
                              ].join(', '),
                            ),
                            onTap: () => openAtroposFile(context, p.path),
                            trailing: IconButton(
                              key: Key('forget-${p.name}'),
                              tooltip: 'Remove from this list',
                              icon: const Icon(Icons.close),
                              onPressed: () => app.forgetPackage(p.path),
                            ),
                          ),
                      ],
                    ),
                  ),
                ],
              ],
            ),
    );
  }
}

class _VaultTile extends StatelessWidget {
  const _VaultTile({
    required this.vault,
    required this.open,
    required this.onTap,
  });

  final KnownVault vault;
  final bool open;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final app = AppScope.read(context);
    final scheme = Theme.of(context).colorScheme;
    final type = FileSystemEntity.typeSync(vault.path);
    final exists = type != FileSystemEntityType.notFound;
    final lfs = type == FileSystemEntityType.directory;
    return Card(
      child: ListTile(
        key: Key('vault-${vault.name}'),
        leading: Icon(
          open ? Icons.lock_open : Icons.lock_outline,
          color: open ? scheme.tertiary : scheme.primary,
        ),
        title: Row(
          children: [
            Flexible(child: Text(vault.name, overflow: TextOverflow.ellipsis)),
            if (lfs) ...[
              const SizedBox(width: 8),
              Container(
                key: Key('lfs-${vault.name}'),
                padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 1),
                decoration: BoxDecoration(
                  color: scheme.secondaryContainer,
                  borderRadius: BorderRadius.circular(6),
                ),
                child: Text(
                  'LFS',
                  style: Theme.of(context).textTheme.labelSmall,
                ),
              ),
            ],
          ],
        ),
        subtitle: Text(
          [
            vault.path,
            if (!exists)
              'Not found. Is the drive connected?'
            else if (vault.lastOpened > 0)
              'Opened ${relativeTime(vault.lastOpened)}',
          ].join('\n'),
          maxLines: 3,
          overflow: TextOverflow.ellipsis,
        ),
        isThreeLine: true,
        onTap: exists ? onTap : null,
        trailing: PopupMenuButton<String>(
          onSelected: (v) async {
            if (v == 'lock') await app.lock(vault.path);
            if (v == 'forget' && context.mounted) {
              final ok = await confirm(
                context,
                title: 'Remove "${vault.name}" from the list?',
                message:
                    'The vault and its files are not touched. You can add '
                    'it again with "Open an existing vault".',
                action: 'Remove',
              );
              if (ok) await app.forget(vault.path);
            }
          },
          itemBuilder: (_) => [
            if (open) const PopupMenuItem(value: 'lock', child: Text('Lock')),
            const PopupMenuItem(
              value: 'forget',
              child: Text('Remove from list'),
            ),
          ],
        ),
      ),
    );
  }
}

class _Intro extends StatelessWidget {
  const _Intro();

  @override
  Widget build(BuildContext context) {
    final t = Theme.of(context).textTheme;
    return Card(
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              children: [
                Icon(
                  Icons.lock_outline,
                  color: Theme.of(context).colorScheme.primary,
                ),
                const SizedBox(width: 8),
                Text('Create your first vault', style: t.titleMedium),
              ],
            ),
            const SizedBox(height: 8),
            const Text(
              'A vault keeps your files encrypted. Without your password, '
              'recovery phrase, fingerprint or security card nobody can see '
              'the files, their names or how big they are.\n\n'
              'A Vault is one .atropos file: easy to copy, upload or send. '
              'A Vault LFS is a folder that encrypts each file on its own, '
              'so very large files stay fast. Put either on a USB stick, a '
              'cloud drive or in a git repository.\n\n'
              'Encryption: ML-KEM-1024 + ECDH P-384 key wrapping, AES-256-GCM, '
              'Argon2id for the password.',
            ),
          ],
        ),
      ),
    );
  }
}

class _Header extends StatelessWidget {
  const _Header(this.text);
  final String text;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.fromLTRB(4, 16, 4, 4),
      child: Text(
        text,
        style: Theme.of(context).textTheme.labelLarge
            ?.copyWith(color: Theme.of(context).colorScheme.primary),
      ),
    );
  }
}

/// Opens an `.atropos` file picked or listed on Home: a vault is unlocked,
/// an older passphrase file is offered an upgrade, and anything else (a
/// file sealed to a vault) is browsed.
Future<void> openAtroposFile(BuildContext context, String path) async {
  final app = AppScope.read(context);
  if (!File(path).existsSync()) {
    final forget = await confirm(
      context,
      title: 'File not found',
      message:
          'It may have been moved, renamed or deleted, or its drive is not '
          'connected. Remove it from the list?',
      action: 'Remove',
    );
    if (forget) await app.forgetPackage(path);
    return;
  }
  if (core.isVault(path: path)) {
    await app.store.remember(path);
    await app.forgetPackage(path);
    await app.refreshVaults();
    if (!context.mounted) return;
    HomePage._openVault(context, path, HomePage._name(path));
    return;
  }
  if (!core.isOldPackage(path: path)) {
    await browsePackageFlow(context, path: path);
    return;
  }
  final choice = await showDialog<String>(
    context: context,
    builder: (ctx) => AlertDialog(
      title: const Text('Upgrade to a vault?'),
      content: const Text(
        'This file was made by an older Atropos. Upgrading turns it into a '
        'full vault in place: its passphrase becomes the vault password, '
        'and it gets a recovery phrase, fingerprint unlock and security '
        'cards. Older versions of Atropos cannot open it afterwards.',
      ),
      actions: [
        TextButton(
          key: const Key('just-look'),
          onPressed: () => Navigator.pop(ctx, 'look'),
          child: const Text('Just look inside'),
        ),
        FilledButton(
          key: const Key('upgrade-go'),
          onPressed: () => Navigator.pop(ctx, 'upgrade'),
          child: const Text('Upgrade'),
        ),
      ],
    ),
  );
  if (choice == null || !context.mounted) return;
  if (choice == 'look') {
    await browsePackageFlow(context, path: path);
    return;
  }
  final pass = await askSecret(
    context,
    title: 'Passphrase of this file',
    label: 'Passphrase',
    action: 'Upgrade',
  );
  if (pass == null || !context.mounted) return;
  final done = await showDialog<core.RekeyDone>(
    context: context,
    barrierDismissible: false,
    builder: (_) => KeysProgressDialog(
      title: 'Upgrading',
      start: () => core.upgradePackage(path: path, passphrase: pass),
    ),
  );
  pass.clear();
  if (done == null || !context.mounted) return;
  await app.forgetPackage(path);
  await app.opened(done.session);
  if (!context.mounted) return;
  await Navigator.of(context).push(
    MaterialPageRoute<void>(
      builder: (_) => RecoveryPhrasePage(
        phrase: done.recoveryPhrase,
        onDone: (ctx) => Navigator.of(ctx).pushReplacement(
          MaterialPageRoute<void>(
            builder: (_) => VaultPage(session: done.session),
          ),
        ),
      ),
    ),
  );
}
