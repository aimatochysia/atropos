import 'dart:io';

import 'package:flutter/material.dart';

import '../core/format.dart';
import '../core/platform_files.dart';
import '../src/rust/api/package.dart' as pkg;
import '../src/rust/api/package.dart' show PackageSession;
import '../src/rust/api/vault.dart' as core;
import '../src/rust/api/vault.dart' show EntryInfo, EntryKind, VaultSession;
import '../src/rust/api/viewer.dart';
import '../state/app_controller.dart';
import 'file_menu.dart';
import 'home_page.dart' show openAtroposFile;
import 'secret_dialog.dart';
import 'storage_access.dart';
import 'task_progress.dart' show plural;
import 'viewer_page.dart';
import 'widgets/feedback.dart';

/// Opens a package for browsing inside Atropos; nothing is written to disk
/// unless a file in it is edited and saved. [path] reopens a package from
/// the recent list; otherwise one is picked. A vault-locked package needs
/// that vault unlocked (pass it as [vault], or it is looked up among the
/// unlocked ones).
Future<void> browsePackageFlow(
  BuildContext context, {
  VaultSession? vault,
  String? path,
}) async {
  final app = AppScope.read(context);
  if (!await ensureStorageAccess(context) || !context.mounted) return;
  final picked = path != null
      ? PickedFiles([path], temporary: false)
      : await app.whilePicking(app.files.pickFileInPlace);
  if (picked == null || !context.mounted) return;
  try {
    final file = picked.paths.first;
    if (path != null && !File(file).existsSync()) {
      final forget = await confirm(
        context,
        title: 'File not found',
        message:
            'It may have been moved, renamed or deleted, or its drive is '
            'not connected. Remove it from the recent list?',
        action: 'Remove',
      );
      if (forget) await app.forgetPackage(file);
      return;
    }
    final info = await guarded(context, () => core.inspectPackage(path: file));
    if (info == null || !context.mounted) return;
    PackageSession? opened;
    if (info.needsPassphrase) {
      final pass = await askSecret(
        context,
        title: 'Passphrase',
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
              'This file can only be opened by the vault it was sealed '
              'to. Unlock that vault first, then open the file again.',
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
    // A temporary copy is gone afterwards: not worth listing or editing.
    if (!picked.temporary) await app.packageUsed(file);
    if (!context.mounted) return;
    await Navigator.of(context).push(
      MaterialPageRoute<void>(
        builder: (_) =>
            PackageBrowserPage(package: opened!, canSave: !picked.temporary),
      ),
    );
  } finally {
    await picked.cleanUp();
  }
}

/// Browses an opened package: folders open in place and files open in the
/// viewer, decrypted straight from the package into memory. Edits rewrite
/// the package in place.
class PackageBrowserPage extends StatefulWidget {
  const PackageBrowserPage({
    super.key,
    required this.package,
    this.canSave = true,
  });
  final PackageSession package;

  /// False when the package is a temporary copy, so edits would be lost.
  final bool canSave;

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

  PackageSession get _p => widget.package;

  /// A passphrase file that can become a vault (checked once).
  late final bool _upgradable =
      widget.canSave && core.isOldPackage(path: _p.path);

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final app = AppScope.of(context);
    if (_app == null) {
      _app = app;
      _reload();
    }
    if (!app.isPackageOpen(_p) && !_closing) {
      _closing = true;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted) Navigator.of(context).popUntil((r) => r.isFirst);
      });
    }
  }

  @override
  void dispose() {
    _app?.closePackage(_p);
    super.dispose();
  }

  Future<void> _reload() async {
    final parent = _crumbs.isEmpty ? null : _crumbs.last.id;
    try {
      final items = await _p.list(
        parent: parent,
        sort: _app!.settings.packageSort,
      );
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

  /// Closes this file and offers to turn it into a vault.
  Future<void> _upgrade() async {
    final path = _p.path;
    final nav = Navigator.of(context);
    final root = nav.context;
    _closing = true;
    _app?.closePackage(_p);
    nav.pop();
    if (root.mounted) await openAtroposFile(root, path);
  }

  Future<void> _sort(core.SortOrder s) async {
    final app = _app!;
    await app.updateSettings(app.settings.copyWith(packageSort: s));
    await _reload();
  }

  ViewerSource get _source {
    final p = _p;
    final app = AppScope.read(context);
    return ViewerSource(
      place: 'package',
      open: (e) =>
          p.openViewer(id: e.id, openAs: app.settings.openAsFor(e.name)),
      isClosed: () => !app.isPackageOpen(p),
      export: _extract,
      exportLabel: 'Extract this file…',
      saveText: widget.canSave
          ? (e, text) => p.saveText(id: e.id, text: text)
          : null,
      saveImage: widget.canSave
          ? (e, v, edits, _) =>
                p.saveEditedImage(viewer: v, id: e.id, edits: edits)
          : null,
      setOpenAs: (e, k) =>
          app.updateSettings(app.settings.withOpenAs(e.name, k)),
      openAsOf: (e) => app.settings.openAsFor(e.name),
    );
  }

  void _view(EntryInfo e) {
    final files = (_items ?? const <EntryInfo>[])
        .where((x) => x.kind == EntryKind.file)
        .toList();
    final i = files.indexWhere((x) => x.id == e.id);
    Navigator.of(context).push(
      MaterialPageRoute<void>(
        builder: (_) => ViewerPage(
          entries: i < 0 ? [e] : files,
          index: i < 0 ? 0 : i,
          source: _source,
          onChanged: _reload,
        ),
      ),
    );
  }

  /// Writes one file out of the package, unencrypted, after a warning.
  Future<void> _extract(EntryInfo e) async {
    final app = AppScope.read(context);
    final ok = await confirm(
      context,
      title: 'Extract unencrypted?',
      message:
          '"${e.name}" will be written to a folder you choose as a normal, '
          'unencrypted file. Anyone with access to that folder can read it.',
      action: 'Choose folder',
      destructive: true,
    );
    if (!ok || !mounted) return;
    if (!await ensureStorageAccess(context) || !mounted) return;
    final dest = await app.whilePicking(
      () => app.files.pickFolder(title: 'Extract here'),
    );
    if (dest == null || !mounted) return;
    await guarded(
      context,
      () => _p.extractFile(id: e.id, destDir: dest),
      success: (path) => 'Wrote $path unencrypted.',
    );
  }

  Future<void> _openAs(EntryInfo e) async {
    final app = AppScope.read(context);
    final k = await pickOpenAs(
      context,
      name: e.name,
      current: app.settings.openAsFor(e.name),
      note:
          'Remembered for every ".${fileExtension(e.name)}" file in sealed files '
          'on this device.',
    );
    if (k == null || !mounted) return;
    if (fileExtension(e.name).isEmpty) {
      showMessage(
        context,
        'This file has no type at the end of its name, so the choice is '
        'only used now.',
      );
    } else {
      await app.updateSettings(
        app.settings.withOpenAs(e.name, k == PreviewKind.none ? null : k),
      );
    }
    if (!mounted) return;
    final files = (_items ?? const <EntryInfo>[])
        .where((x) => x.kind == EntryKind.file)
        .toList();
    final i = files.indexWhere((x) => x.id == e.id);
    final source = _source;
    await Navigator.of(context).push(
      MaterialPageRoute<void>(
        builder: (_) => ViewerPage(
          entries: i < 0 ? [e] : files,
          index: i < 0 ? 0 : i,
          source: ViewerSource(
            place: source.place,
            // Without an extension the choice cannot be stored: use it once.
            open: (x) => x.id == e.id && k != PreviewKind.none
                ? _p.openViewer(id: x.id, openAs: k)
                : source.open(x),
            isClosed: source.isClosed,
            export: source.export,
            exportLabel: source.exportLabel,
            saveText: source.saveText,
            saveImage: source.saveImage,
            setOpenAs: source.setOpenAs,
            openAsOf: source.openAsOf,
          ),
          onChanged: _reload,
        ),
      ),
    );
  }

  void _menu(EntryInfo e) {
    if (e.kind == EntryKind.link) return;
    final app = AppScope.read(context);
    showModalBottomSheet<void>(
      context: context,
      builder: (ctx) {
        void run(Future<void> Function() f) {
          Navigator.pop(ctx);
          f();
        }

        final folder = e.kind == EntryKind.folder;
        return SafeArea(
          child: SingleChildScrollView(
            child: Column(
              mainAxisSize: MainAxisSize.min,
              children: [
                ListTile(
                  leading: Icon(_icon(e)),
                  title: Text(e.name),
                  subtitle: Text(_subtitle(e)),
                ),
                const Divider(height: 1),
                if (folder)
                  ListTile(
                    leading: const Icon(Icons.folder_open_outlined),
                    title: const Text('Open'),
                    onTap: () {
                      Navigator.pop(ctx);
                      _enter(e);
                    },
                  )
                else ...[
                  ListTile(
                    key: const Key('item-view'),
                    leading: const Icon(Icons.visibility_outlined),
                    title: const Text('View'),
                    onTap: () {
                      Navigator.pop(ctx);
                      _view(e);
                    },
                  ),
                  ListTile(
                    key: const Key('item-open-as'),
                    leading: const Icon(Icons.file_open_outlined),
                    title: const Text('Open as…'),
                    subtitle: Text(
                      previewLabel(
                        effectiveKind(e, rule: app.settings.openAsFor(e.name)),
                      ),
                    ),
                    onTap: () => run(() => _openAs(e)),
                  ),
                  ListTile(
                    key: const Key('item-extract'),
                    leading: const Icon(Icons.file_download_outlined),
                    title: const Text('Extract this file…'),
                    onTap: () => run(() => _extract(e)),
                  ),
                ],
                ListTile(
                  key: const Key('item-details'),
                  leading: const Icon(Icons.info_outline),
                  title: const Text('Details'),
                  onTap: () => run(
                    () => showEntryDetails(
                      context,
                      e,
                      location: _crumbs.map((c) => c.name).join(' / '),
                      rule: app.settings.openAsFor(e.name),
                    ),
                  ),
                ),
              ],
            ),
          ),
        );
      },
    );
  }

  IconData _icon(EntryInfo e) => e.kind == EntryKind.folder
      ? Icons.folder_outlined
      : e.kind == EntryKind.link
      ? Icons.link
      : previewIcon(effectiveKind(e, rule: _app?.settings.openAsFor(e.name)));

  static String _subtitle(EntryInfo e) => e.kind == EntryKind.folder
      ? plural(e.children, 'item')
      : [
          formatBytes(e.size.toInt()),
          if (e.modified > 0) relativeTime(e.modified),
        ].join(' · ');

  @override
  Widget build(BuildContext context) {
    final items = _items;
    final app = AppScope.of(context);
    final title = _crumbs.isEmpty ? _p.name : _crumbs.last.name;
    return PopScope(
      canPop: _crumbs.isEmpty,
      onPopInvokedWithResult: (didPop, _) {
        if (!didPop && _crumbs.isNotEmpty) _up();
      },
      child: Scaffold(
        appBar: AppBar(
          title: Text(title, overflow: TextOverflow.ellipsis),
          actions: [
            if (_upgradable)
              TextButton.icon(
                key: const Key('package-upgrade'),
                onPressed: _upgrade,
                icon: const Icon(Icons.upgrade),
                label: const Text('Upgrade'),
              ),
            SortButton(sort: app.settings.packageSort, onChanged: _sort),
          ],
          bottom: PreferredSize(
            preferredSize: const Size.fromHeight(28),
            child: Padding(
              padding: const EdgeInsets.only(bottom: 8, left: 16, right: 16),
              child: Text(
                widget.canSave
                    ? 'Older .atropos file. Files are decrypted in memory '
                          'only; saving an edit rewrites the file.'
                    : 'Read-only copy. Files are decrypted in memory only.',
                key: const Key('package-readonly'),
                textAlign: TextAlign.center,
              ),
            ),
          ),
        ),
        body: _error != null
            ? Center(
                child: Padding(
                  padding: const EdgeInsets.all(24),
                  child: Text(
                    'This file could not be read.',
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
                  return GestureDetector(
                    onSecondaryTap: () => _menu(e),
                    child: ListTile(
                      key: Key('package-item-${e.name}'),
                      leading: Icon(_icon(e)),
                      title: Text(e.name, overflow: TextOverflow.ellipsis),
                      subtitle: Text(_subtitle(e)),
                      trailing: e.kind == EntryKind.link
                          ? null
                          : IconButton(
                              key: Key('package-more-${e.name}'),
                              tooltip: 'More',
                              icon: const Icon(Icons.more_vert),
                              onPressed: () => _menu(e),
                            ),
                      onTap: e.kind == EntryKind.link
                          ? null
                          : folder
                          ? () => _enter(e)
                          : () => _view(e),
                      onLongPress: () => _menu(e),
                    ),
                  );
                },
              ),
      ),
    );
  }
}
