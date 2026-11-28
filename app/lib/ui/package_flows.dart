import 'package:flutter/material.dart';

import '../core/format.dart';
import '../core/platform_files.dart';
import '../src/rust/api/vault.dart' as core;
import '../src/rust/api/vault.dart' show OutcomeInfo, VaultSession;
import '../state/app_controller.dart';
import 'secret_dialog.dart';
import 'secret_field.dart' show SecretInput;
import 'storage_access.dart';
import 'task_progress.dart';
import 'widgets/feedback.dart';

String _sealedSummary(OutcomeInfo o) =>
    'Sealed ${plural(o.files, 'file')} (${formatBytes(o.bytes.toInt())}) '
    'into one encrypted file.';

String _openedSummary(OutcomeInfo o) =>
    'Opened ${plural(o.files, 'file')} and ${plural(o.folders, 'folder')} '
    '(${formatBytes(o.bytes.toInt())}).';

/// Lets the person choose files or a whole folder from disk.
Future<PickedFiles?> pickSources(BuildContext context) async {
  final app = AppScope.read(context);
  final choice = await showModalBottomSheet<String>(
    context: context,
    builder: (ctx) => SafeArea(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          ListTile(
            key: const Key('pick-files'),
            leading: const Icon(Icons.insert_drive_file_outlined),
            title: const Text('Files'),
            onTap: () => Navigator.pop(ctx, 'files'),
          ),
          ListTile(
            key: const Key('pick-folder'),
            leading: const Icon(Icons.folder_outlined),
            title: const Text('A whole folder'),
            onTap: () => Navigator.pop(ctx, 'folder'),
          ),
        ],
      ),
    ),
  );
  if (choice == null) return null;
  if (choice == 'files') {
    return app.whilePicking(app.files.pickFiles);
  }
  final dir = await app.whilePicking(app.files.pickFolder);
  return dir == null ? null : PickedFiles([dir], temporary: false);
}

String _baseName(String path) {
  final parts = path.split(RegExp(r'[\\/]')).where((p) => p.isNotEmpty);
  return parts.isEmpty ? path : parts.last;
}

/// Home screen: seal files from disk into a passphrase-locked package.
Future<void> sealFilesFromDisk(BuildContext context) async {
  final app = AppScope.read(context);
  if (!await ensureStorageAccess(context) || !context.mounted) return;
  final picked = await pickSources(context);
  if (picked == null || !context.mounted) return;
  try {
    final first = _baseName(picked.paths.first);
    final out = await app.whilePicking(
      () => app.files.pickSaveFile(
        picked.paths.length == 1 ? '$first.atropos' : 'sealed.atropos',
      ),
    );
    if (out == null || !context.mounted) return;
    final pass = await askSecret(
      context,
      title: 'Choose a passphrase',
      message:
          'Anyone with this passphrase can open the file. Share it '
          'separately from the file, for example by phone.',
      action: 'Seal',
      isNew: true,
    );
    if (pass == null || !context.mounted) return;
    await runTask(
      context,
      title: 'Sealing',
      start: () =>
          core.sealFiles(sources: picked.paths, outFile: out, passphrase: pass),
      summary: _sealedSummary,
    );
    pass.clear();
  } finally {
    await picked.cleanUp();
  }
}

/// Home screen: open a passphrase-locked package into a folder on disk.
Future<void> extractPackageToDisk(BuildContext context) async {
  final app = AppScope.read(context);
  if (!await ensureStorageAccess(context) || !context.mounted) return;
  final picked = await app.whilePicking(app.files.pickFile);
  if (picked == null || !context.mounted) return;
  try {
    final file = picked.paths.first;
    final info = await guarded(context, () => core.inspectPackage(path: file));
    if (info == null || !context.mounted) return;
    if (!info.needsPassphrase) {
      await confirm(
        context,
        title: 'Locked to a vault',
        message:
            'This file can only be opened by the vault it was sealed '
            'to. Unlock that vault, then choose "Add a sealed file here" in its menu.',
      );
      return;
    }
    final dest = await app.whilePicking(
      () => app.files.pickFolder(title: 'Extract here'),
    );
    if (dest == null || !context.mounted) return;
    final ok = await confirm(
      context,
      title: 'Write decrypted files?',
      message:
          'The contents will be saved unencrypted in\n$dest\n\n'
          'Anyone with access to that folder can read them.',
      action: 'Extract',
      destructive: true,
    );
    if (!ok || !context.mounted) return;
    final pass = await askSecret(context, title: 'Passphrase', action: 'Open');
    if (pass == null || !context.mounted) return;
    await runTask(
      context,
      title: 'Opening',
      start: () =>
          core.extractFiles(packageFile: file, passphrase: pass, destDir: dest),
      summary: _openedSummary,
    );
    pass.clear();
  } finally {
    await picked.cleanUp();
  }
}

/// Vault: seal selected items into a package locked to this vault or to a
/// passphrase.
Future<void> sealFromVault(
  BuildContext context,
  VaultSession session,
  List<String> ids,
  String suggestedName,
) async {
  final app = AppScope.read(context);
  final mode = await showDialog<String>(
    context: context,
    builder: (ctx) => SimpleDialog(
      title: const Text('Who can open the sealed file?'),
      children: [
        SimpleDialogOption(
          key: const Key('seal-to-vault'),
          onPressed: () => Navigator.pop(ctx, 'vault'),
          child: const ListTile(
            leading: Icon(Icons.lock_outline),
            title: Text('Only this vault'),
            subtitle: Text(
              'For carrying files between your devices or backing them up. '
              'No passphrase needed; this vault opens it.',
            ),
          ),
        ),
        SimpleDialogOption(
          key: const Key('seal-to-passphrase'),
          onPressed: () => Navigator.pop(ctx, 'passphrase'),
          child: const ListTile(
            leading: Icon(Icons.key_outlined),
            title: Text('Anyone with a passphrase'),
            subtitle: Text('For sharing with someone else who has Atropos.'),
          ),
        ),
      ],
    ),
  );
  if (mode == null || !context.mounted) return;
  if (!await ensureStorageAccess(context) || !context.mounted) return;
  final out = await app.whilePicking(
    () => app.files.pickSaveFile('$suggestedName.atropos'),
  );
  if (out == null || !context.mounted) return;
  SecretInput? pass;
  if (mode == 'passphrase') {
    pass = await askSecret(
      context,
      title: 'Choose a passphrase',
      message: 'Share it separately from the file.',
      action: 'Seal',
      isNew: true,
    );
    if (pass == null || !context.mounted) return;
  }
  await runTask(
    context,
    title: 'Sealing',
    start: () => session.sealPackage(ids: ids, outFile: out, passphrase: pass),
    summary: _sealedSummary,
  );
  pass?.clear();
}

/// Vault: open a package into the current folder of this vault.
Future<bool> openPackageIntoVault(
  BuildContext context,
  VaultSession session,
  String? parent,
) async {
  final app = AppScope.read(context);
  if (!await ensureStorageAccess(context) || !context.mounted) return false;
  final picked = await app.whilePicking(app.files.pickFile);
  if (picked == null || !context.mounted) return false;
  try {
    final file = picked.paths.first;
    final info = await guarded(context, () => core.inspectPackage(path: file));
    if (info == null || !context.mounted) return false;
    SecretInput? pass;
    if (info.needsPassphrase) {
      pass = await askSecret(context, title: 'Passphrase', action: 'Open');
      if (pass == null || !context.mounted) return false;
    }
    final o = await runTask(
      context,
      title: 'Adding to the vault',
      start: () => session.openPackage(
        packageFile: file,
        passphrase: pass,
        parent: parent,
      ),
      summary: _openedSummary,
    );
    pass?.clear();
    return o != null;
  } finally {
    await picked.cleanUp();
  }
}
