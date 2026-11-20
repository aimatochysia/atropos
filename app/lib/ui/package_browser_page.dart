import 'package:flutter/material.dart';

import '../core/format.dart';
import '../src/rust/api/package.dart' as pkg;
import '../src/rust/api/package.dart' show PackageSession;
import '../src/rust/api/vault.dart' as core;
import '../src/rust/api/vault.dart' show EntryInfo, EntryKind, VaultSession;
import '../src/rust/api/viewer.dart';
import '../state/app_controller.dart';
import 'secret_dialog.dart';
import 'storage_access.dart';
import 'task_progress.dart' show plural;
import 'viewer_page.dart';
import 'widgets/feedback.dart';

/// Picks a package and opens it for browsing inside Atropos; nothing is
/// written to disk. A vault-locked package needs that vault unlocked
/// (pass it as [vault], or it is looked up among the unlocked ones).
Future<void> browsePackageFlow(
  BuildContext context, {
  VaultSession? vault,
}) async {
  final app = AppScope.read(context);
  if (!await ensureStorageAccess(context) || !context.mounted) return;
  final picked = await app.whilePicking(app.files.pickFile);
  if (picked == null || !context.mounted) return;
  try {
    final file = picked.paths.first;
    final info = await guarded(context, () => core.inspectPackage(path: file));
    if (info == null || !context.mounted) return;
    PackageSession? opened;
    if (info.needsPassphrase) {
      final pass = await askSecret(
        context,
        title: 'Package passphrase',
        action: 'Open',
      );
      if (pass == null || !context.mounted) return;
      opened = await guarded(
        context,
        () => pkg.browsePackage(path: file, passphrase: pass),
      );
      pass.clear();
    } else {
      final key = vault?.vaultId == info.vaultId
          ? vault
          : app.sessionForVaultId(info.vaultId);
      if (key == null) {
        await confirm(
          context,
          title: 'Locked to a vault',
          message:
              'This package can only be opened by the vault it was sealed '
              'to. Unlock that vault first, then open the package again.',
        );
        return;
      }
      opened = await guarded(context, () => key.browsePackage(path: file));
    }
    if (opened == null || !context.mounted) {
      opened?.close();
      return;
    }
    app.packageOpened(opened);
    await Navigator.of(context).push(
      MaterialPageRoute<void>(
        builder: (_) => PackageBrowserPage(package: opened!),
      ),
    );
  } finally {
    await picked.cleanUp();
  }
}

/// Browses an opened package: folders open in place and files open in the
/// viewer, decrypted straight from the package into memory.
class PackageBrowserPage extends StatefulWidget {
  const PackageBrowserPage({super.key, required this.package});
  final PackageSession package;

  @override
  State<PackageBrowserPage> createState() => _PackageBrowserPageState();
}

class _Crumb {
  const _Crumb(this.id, this.name);
  final String id;
  final String name;
}

class _PackageBrowserPageState extends State<PackageBrowserPage> {
  AppController? _app;
  final List<_Crumb> _crumbs = [];
  List<EntryInfo>? _items;
  Object? _error;
  bool _closing = false;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final app = AppScope.of(context);
    if (_app == null) {
      _app = app;
      _reload();
    }
    if (!app.isPackageOpen(widget.package) && !_closing) {
      _closing = true;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted) Navigator.of(context).popUntil((r) => r.isFirst);
      });
    }
  }

  @override
  void dispose() {
    _app?.closePackage(widget.package);
    super.dispose();
  }

  Future<void> _reload() async {
    final parent = _crumbs.isEmpty ? null : _crumbs.last.id;
    try {
      final items = await widget.package.list(parent: parent);
      if (mounted) setState(() => _items = items);
    } catch (e) {
      if (mounted) setState(() => _error = e);
    }
  }

  void _enter(EntryInfo e) {
    setState(() {
      _crumbs.add(_Crumb(e.id, e.name));
      _items = null;
    });
    _reload();
  }

  void _up() {
    setState(() {
      _crumbs.removeLast();
      _items = null;
    });
    _reload();
  }

  void _view(EntryInfo e) {
    final p = widget.package;
    Navigator.of(context).push(
      MaterialPageRoute<void>(
        builder: (_) => ViewerPage(
          entry: e,
          open: () => p.openViewer(id: e.id),
          isClosed: () => !AppScope.read(context).isPackageOpen(p),
        ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final items = _items;
    final title = _crumbs.isEmpty ? widget.package.name : _crumbs.last.name;
    return PopScope(
      canPop: _crumbs.isEmpty,
      onPopInvokedWithResult: (didPop, _) {
        if (!didPop && _crumbs.isNotEmpty) _up();
      },
      child: Scaffold(
        appBar: AppBar(
          title: Text(title, overflow: TextOverflow.ellipsis),
          bottom: const PreferredSize(
            preferredSize: Size.fromHeight(28),
            child: Padding(
              padding: EdgeInsets.only(bottom: 8),
              child: Text(
                'Read-only. Files are decrypted in memory only.',
                key: Key('package-readonly'),
              ),
            ),
          ),
        ),
        body: _error != null
            ? Center(
                child: Padding(
                  padding: const EdgeInsets.all(24),
                  child: Text(
                    'This package could not be read.',
                    textAlign: TextAlign.center,
                    style: TextStyle(
                      color: Theme.of(context).colorScheme.error,
                    ),
                  ),
                ),
              )
            : items == null
            ? const Center(child: CircularProgressIndicator())
            : items.isEmpty
            ? const Center(child: Text('This folder is empty.'))
            : ListView.builder(
                key: const Key('package-list'),
                itemCount: items.length,
                itemBuilder: (context, i) {
                  final e = items[i];
                  final folder = e.kind == EntryKind.folder;
                  return ListTile(
                    key: Key('package-item-${e.name}'),
                    leading: Icon(
                      folder
                          ? Icons.folder_outlined
                          : e.kind == EntryKind.link
                          ? Icons.link
                          : previewIcon(previewForName(name: e.name)),
                    ),
                    title: Text(e.name, overflow: TextOverflow.ellipsis),
                    subtitle: Text(
                      folder
                          ? plural(e.children, 'item')
                          : [
                              formatBytes(e.size.toInt()),
                              if (e.modified > 0) relativeTime(e.modified),
                            ].join(' · '),
                    ),
                    trailing: folder ? const Icon(Icons.chevron_right) : null,
                    onTap: e.kind == EntryKind.link
                        ? null
                        : folder
                        ? () => _enter(e)
                        : () => _view(e),
                  );
                },
              ),
      ),
    );
  }
}
