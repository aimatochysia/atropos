import 'package:flutter/material.dart';

import '../state/app_controller.dart';

/// On Android, makes sure Atropos may read and write normal folders. Shows
/// why it is needed and opens the system setting; returns false until the
/// person has allowed it.
Future<bool> ensureStorageAccess(BuildContext context) async {
  final files = AppScope.read(context).files;
  if (await files.hasStorageAccess()) return true;
  if (!context.mounted) return false;
  final go = await showDialog<bool>(
    context: context,
    builder: (ctx) => AlertDialog(
      title: const Text('Allow access to your files'),
      content: const Text(
        'Vaults are normal folders, so you can keep them on a USB stick, '
        'in a synced cloud folder or in a git repository. Android only lets '
        'Atropos read and write such folders with "All files access".\n\n'
        'Atropos only touches the folders and files you choose.',
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.pop(ctx, false),
          child: const Text('Not now'),
        ),
        FilledButton(
          onPressed: () => Navigator.pop(ctx, true),
          child: const Text('Open settings'),
        ),
      ],
    ),
  );
  if (go == true) await files.requestStorageAccess();
  return false;
}
