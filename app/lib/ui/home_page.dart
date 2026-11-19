import 'dart:io';

import 'package:flutter/material.dart';

import '../core/crypto_service.dart';
import '../core/format.dart';
import '../src/rust/api/vault.dart' as core;
import '../state/app_controller.dart';
import 'create_vault_page.dart';
import 'package_flows.dart';
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
    if (!await ensureStorageAccess(context)) return;
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

  static String _name(String path) =>
      path.split(RegExp(r'[\\/]')).where((p) => p.isNotEmpty).lastOrNull ??
      path;

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
                  label: const Text('Open an existing vault folder'),
                ),
                const _Header('Packages'),
                Card(
                  child: Column(
                    children: [
                      ListTile(
                        key: const Key('seal-files'),
                        leading: const Icon(Icons.inventory_2_outlined),
                        title: const Text('Seal files into a package'),
                        subtitle: const Text(
                          'One encrypted .atropos file, opened with a '
                          'passphrase. Good for sending or uploading.',
                        ),
                        onTap: () => sealFilesFromDisk(context),
                      ),
                      const Divider(height: 1),
                      ListTile(
                        key: const Key('extract-package'),
                        leading: const Icon(Icons.unarchive_outlined),
                        title: const Text('Open a package'),
                        subtitle: const Text(
                          'Decrypt a passphrase-locked package into a folder.',
                        ),
                        onTap: () => extractPackageToDisk(context),
                      ),
                    ],
                  ),
                ),
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
    final exists = Directory(vault.path).existsSync();
    return Card(
      child: ListTile(
        key: Key('vault-${vault.name}'),
        leading: Icon(
          open ? Icons.lock_open : Icons.lock_outline,
          color: open ? scheme.tertiary : scheme.primary,
        ),
        title: Text(vault.name),
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
                    'The vault folder and its files are not touched. You can '
                    'add it again with "Open an existing vault folder".',
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
              'A vault is a normal folder full of encrypted pieces. Put it on '
              'a USB stick, a cloud drive or in a git repository: without your '
              'password or recovery phrase nobody can see the files, their '
              'names or how big they are.\n\n'
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
