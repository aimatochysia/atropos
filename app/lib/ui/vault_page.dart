import 'dart:io';

import 'package:flutter/material.dart';

import '../core/biometrics.dart';
import '../core/format.dart';
import '../core/platform_files.dart';
import '../src/rust/api/vault.dart';
import '../src/rust/api/viewer.dart';
import '../state/app_controller.dart';
import 'card_flows.dart';
import 'check_page.dart';
import 'file_menu.dart';
import 'package_browser_page.dart';
import 'package_flows.dart';
import 'recovery_page.dart';
import 'secret_dialog.dart';
import 'storage_access.dart';
import 'task_progress.dart';
import 'viewer_page.dart';
import 'widgets/feedback.dart';

/// Browses an unlocked vault: folders, files, adding, exporting, packages.
class VaultPage extends StatefulWidget {
  const VaultPage({super.key, required this.session});

  final VaultSession session;

  @override
  State<VaultPage> createState() => _VaultPageState();
}

class _Crumb {
  const _Crumb(this.id, this.name);
  final String id;
  final String name;
}

class _VaultPageState extends State<VaultPage> {
  final List<_Crumb> _crumbs = [];
  final Set<String> _selected = {};
  List<EntryInfo> _entries = const [];
  bool _loading = true;
  bool _closing = false;
  AppController? _app;

  VaultSession get _s => widget.session;
  String? get _parent => _crumbs.isEmpty ? null : _crumbs.last.id;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final app = AppScope.of(context);
    if (_app == null) {
      _app = app;
      _reload();
    }
    if (!app.isOpen(_s.path) && !_closing) {
      _closing = true;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted) Navigator.of(context).popUntil((r) => r.isFirst);
      });
    }
  }

  Future<void> _reload() async {
    try {
      final list = await _s.list(
        parent: _parent,
        sort: AppScope.read(context).sortFor(_s.path),
      );
      if (!mounted) return;
      setState(() {
        _entries = list;
        _loading = false;
        _selected.removeWhere((id) => !list.any((e) => e.id == id));
      });
    } catch (e) {
      if (!mounted || _closing) return;
      setState(() => _loading = false);
      showMessage(context, e.toString(), error: true);
    }
  }

  void _open(EntryInfo e) {
    if (_selected.isNotEmpty) {
      _toggle(e);
      return;
    }
    if (e.kind == EntryKind.folder) {
      setState(() {
        _crumbs.add(_Crumb(e.id, e.name));
        _loading = true;
      });
      _reload();
    } else if (e.kind == EntryKind.file &&
        effectiveKind(e) != PreviewKind.none) {
      _view(e);
    } else {
      _details(e);
    }
  }

  void _view(EntryInfo e, {List<EntryInfo>? among}) {
    final files = (among ?? _entries)
        .where((x) => x.kind == EntryKind.file)
        .toList();
    final i = files.indexWhere((x) => x.id == e.id);
    Navigator.of(context).push(
      MaterialPageRoute<void>(
        builder: (_) => ViewerPage(
          entries: i < 0 ? [e] : files,
          index: i < 0 ? 0 : i,
          source: vaultSource(_s, export: (x) => _export([x])),
          onChanged: _reload,
        ),
      ),
    );
  }

  Future<void> _openAs(EntryInfo e) async {
    final k = await pickOpenAs(
      context,
      name: e.name,
      current: e.openAs,
      note: 'Remembered for this file inside the vault.',
    );
    if (k == null || !mounted) return;
    final kind = k == PreviewKind.none ? null : k;
    await guarded(context, () => _s.setOpenAs(id: e.id, openAs: kind));
    await _reload();
    if (!mounted) return;
    final fresh = _entries.where((x) => x.id == e.id).firstOrNull ?? e;
    if (effectiveKind(fresh) != PreviewKind.none) _view(fresh);
  }

  Future<void> _sort(SortOrder s) async {
    await AppScope.read(context).setSort(_s.path, s);
    await _reload();
  }

  Future<void> _search() async {
    final hit = await Navigator.of(context).push<SearchHit>(
      MaterialPageRoute(builder: (_) => VaultSearchPage(session: _s)),
    );
    if (hit == null || !mounted) return;
    setState(() {
      _crumbs
        ..clear()
        ..addAll(hit.folders.map((f) => _Crumb(f.id, f.name)));
      _selected.clear();
      _loading = true;
    });
    await _reload();
    if (!mounted) return;
    final e = hit.entry;
    if (e.kind == EntryKind.folder) {
      _open(e);
    } else if (e.kind == EntryKind.file &&
        effectiveKind(e) != PreviewKind.none) {
      _view(e);
    } else {
      _details(e);
    }
  }

  void _goTo(int depth) {
    if (depth == _crumbs.length) return;
    setState(() {
      _crumbs.removeRange(depth, _crumbs.length);
      _selected.clear();
      _loading = true;
    });
    _reload();
  }

  void _toggle(EntryInfo e) => setState(() {
    if (!_selected.remove(e.id)) _selected.add(e.id);
  });

  List<EntryInfo> get _selectedEntries =>
      _entries.where((e) => _selected.contains(e.id)).toList();

  // ---- adding ------------------------------------------------------------

  Future<void> _add() async {
    final choice = await showModalBottomSheet<String>(
      context: context,
      builder: (ctx) => SafeArea(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            ListTile(
              key: const Key('add-files'),
              leading: const Icon(Icons.upload_file),
              title: const Text('Add files'),
              onTap: () => Navigator.pop(ctx, 'files'),
            ),
            ListTile(
              key: const Key('add-folder'),
              leading: const Icon(Icons.drive_folder_upload_outlined),
              title: const Text('Add a folder'),
              onTap: () => Navigator.pop(ctx, 'folder'),
            ),
            ListTile(
              key: const Key('new-folder'),
              leading: const Icon(Icons.create_new_folder_outlined),
              title: const Text('New empty folder'),
              onTap: () => Navigator.pop(ctx, 'new'),
            ),
            ListTile(
              key: const Key('open-package'),
              leading: const Icon(Icons.inventory_2_outlined),
              title: const Text('Open a package here'),
              onTap: () => Navigator.pop(ctx, 'package'),
            ),
          ],
        ),
      ),
    );
    if (!mounted || choice == null) return;
    switch (choice) {
      case 'new':
        await _newFolder();
      case 'package':
        if (await openPackageIntoVault(context, _s, _parent)) _reload();
      default:
        await _import(folder: choice == 'folder');
    }
  }

  Future<void> _newFolder() async {
    final name = await promptText(
      context,
      title: 'New folder',
      label: 'Name',
      action: 'Create',
    );
    if (name == null || name.isEmpty || !mounted) return;
    await guarded(context, () => _s.createFolder(parent: _parent, name: name));
    _reload();
  }

  Future<void> _import({required bool folder}) async {
    final app = AppScope.read(context);
    if (!await ensureStorageAccess(context) || !mounted) return;
    final PickedFiles? picked;
    if (folder) {
      final dir = await app.whilePicking(app.files.pickFolder);
      picked = dir == null ? null : PickedFiles([dir], temporary: false);
    } else {
      picked = await app.whilePicking(app.files.pickFiles);
    }
    if (picked == null || !mounted) return;
    try {
      var deleteOriginals = false;
      if (!picked.temporary) {
        final answer = await _askDeleteOriginals(picked.paths.length);
        if (answer == null || !mounted) return;
        deleteOriginals = answer;
      }
      final o = await runTask(
        context,
        title: 'Encrypting into ${_s.name}',
        start: () => _s.import_(
          sources: picked!.paths,
          parent: _parent,
          deleteOriginals: deleteOriginals,
        ),
        summary: (o) {
          final what =
              'Encrypted ${plural(o.files, 'file')} '
              '(${formatBytes(o.bytes.toInt())}).';
          if (o.originalsDeleted) {
            return '$what Each file was read back and checked, then the '
                'originals were deleted.';
          }
          return picked!.temporary
              ? '$what The originals are still where they were; delete them '
                    'yourself if you only want the encrypted copy.'
              : what;
        },
      );
      if (o != null) _reload();
    } finally {
      await picked.cleanUp();
    }
  }

  Future<bool?> _askDeleteOriginals(int count) {
    var delete = false;
    return showDialog<bool>(
      context: context,
      builder: (ctx) => StatefulBuilder(
        builder: (ctx, set) => AlertDialog(
          title: const Text('Add to the vault'),
          content: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              const Text(
                'The files are encrypted into the vault. The unencrypted '
                'originals stay on disk unless you remove them.',
              ),
              CheckboxListTile(
                key: const Key('delete-originals'),
                contentPadding: EdgeInsets.zero,
                value: delete,
                onChanged: (v) => set(() => delete = v ?? false),
                title: const Text('Delete the originals afterwards'),
                subtitle: const Text(
                  'Only after each file was read back from the vault and '
                  'matched. SSDs and flash may keep old copies; this is not '
                  'a secure wipe.',
                ),
              ),
            ],
          ),
          actions: [
            TextButton(
              onPressed: () => Navigator.pop(ctx),
              child: const Text('Cancel'),
            ),
            FilledButton(
              key: const Key('import-go'),
              onPressed: () => Navigator.pop(ctx, delete),
              child: const Text('Encrypt'),
            ),
          ],
        ),
      ),
    );
  }

  // ---- item actions ------------------------------------------------------

  Future<void> _export(List<EntryInfo> items) async {
    if (items.isEmpty) return;
    final app = AppScope.read(context);
    final bulk = items.length > 1 || items.first.kind == EntryKind.folder;
    final ok = bulk
        ? await _confirmBulkExport(items)
        : await confirm(
            context,
            title: 'Export unencrypted?',
            message:
                '"${items.first.name}" will be written to a folder you choose '
                'as a normal, unencrypted file. Anyone with access to that '
                'folder can read it.',
            action: 'Choose folder',
            destructive: true,
          );
    if (!ok || !mounted) return;
    if (!await ensureStorageAccess(context) || !mounted) return;
    final dest = await app.whilePicking(
      () => app.files.pickFolder(title: 'Export here'),
    );
    if (dest == null || !mounted) return;
    await runTask(
      context,
      title: 'Exporting',
      start: () =>
          _s.export_(ids: items.map((e) => e.id).toList(), destDir: dest),
      summary: (o) =>
          'Wrote ${plural(o.files, 'file')} (${formatBytes(o.bytes.toInt())}) '
          'unencrypted to $dest.',
    );
    if (mounted) setState(_selected.clear);
  }

  /// Exporting many files at once asks for the vault's name to be typed,
  /// so it cannot happen by accident.
  Future<bool> _confirmBulkExport(List<EntryInfo> items) async {
    final typed = await promptText(
      context,
      title: 'Export unencrypted?',
      label: 'Type "${_s.name}" to confirm',
      action: 'Export',
      hint: _s.name,
    );
    if (typed == null) return false;
    if (typed != _s.name) {
      if (mounted) {
        showMessage(context, 'The name did not match. Nothing was exported.');
      }
      return false;
    }
    return true;
  }

  Future<void> _rename(EntryInfo e) async {
    final name = await promptText(
      context,
      title: 'Rename',
      label: 'Name',
      initial: e.name,
      action: 'Rename',
    );
    if (name == null || name.isEmpty || name == e.name || !mounted) return;
    await guarded(context, () => _s.rename(id: e.id, name: name));
    _reload();
  }

  Future<void> _delete(List<EntryInfo> items) async {
    if (items.isEmpty) return;
    final what = items.length == 1
        ? '"${items.first.name}"'
        : '${items.length} items';
    final ok = await confirm(
      context,
      title: 'Delete $what?',
      message:
          'They are removed from the vault and their encrypted data is '
          'deleted. This cannot be undone.',
      action: 'Delete',
      destructive: true,
    );
    if (!ok || !mounted) return;
    await guarded(
      context,
      () => _s.delete(ids: items.map((e) => e.id).toList()),
    );
    if (mounted) setState(_selected.clear);
    _reload();
  }

  Future<void> _move(List<EntryInfo> items) async {
    if (items.isEmpty) return;
    final target = await showDialog<_Crumb>(
      context: context,
      builder: (_) =>
          _FolderChooser(session: _s, excluded: items.map((e) => e.id).toSet()),
    );
    if (target == null || !mounted) return;
    await guarded(
      context,
      () => _s.moveTo(
        ids: items.map((e) => e.id).toList(),
        parent: target.id.isEmpty ? null : target.id,
      ),
      success: (_) => 'Moved to ${target.name}.',
    );
    if (mounted) setState(_selected.clear);
    _reload();
  }

  Future<void> _seal(List<EntryInfo> items) async {
    if (items.isEmpty) return;
    await sealFromVault(
      context,
      _s,
      items.map((e) => e.id).toList(),
      items.length == 1 ? items.first.name : _s.name,
    );
    if (mounted) setState(_selected.clear);
  }

  void _details(EntryInfo e) {
    showModalBottomSheet<void>(
      context: context,
      builder: (ctx) {
        void run(Future<void> Function() f) {
          Navigator.pop(ctx);
          f();
        }

        return SafeArea(
          child: SingleChildScrollView(
            child: Column(
              mainAxisSize: MainAxisSize.min,
              children: [
                ListTile(
                  leading: _iconFor(e, Theme.of(ctx).colorScheme),
                  title: Text(e.name),
                  subtitle: Text(_subtitle(e)),
                ),
                const Divider(height: 1),
                if (e.kind == EntryKind.file) ...[
                  ListTile(
                    key: const Key('item-view'),
                    leading: const Icon(Icons.visibility_outlined),
                    title: const Text('View'),
                    subtitle: Text(describeFileType(name: e.name)),
                    onTap: () {
                      Navigator.pop(ctx);
                      _view(e);
                    },
                  ),
                  ListTile(
                    key: const Key('item-open-as'),
                    leading: const Icon(Icons.file_open_outlined),
                    title: const Text('Open as…'),
                    subtitle: Text(previewLabel(effectiveKind(e))),
                    onTap: () => run(() => _openAs(e)),
                  ),
                ],
                ListTile(
                  key: const Key('item-select'),
                  leading: const Icon(Icons.check_box_outlined),
                  title: const Text('Select'),
                  onTap: () {
                    Navigator.pop(ctx);
                    _toggle(e);
                  },
                ),
                ListTile(
                  key: const Key('item-export'),
                  leading: const Icon(Icons.file_download_outlined),
                  title: const Text('Export unencrypted…'),
                  onTap: () => run(() => _export([e])),
                ),
                ListTile(
                  leading: const Icon(Icons.inventory_2_outlined),
                  title: const Text('Seal into a package…'),
                  onTap: () => run(() => _seal([e])),
                ),
                ListTile(
                  leading: const Icon(Icons.drive_file_rename_outline),
                  title: const Text('Rename'),
                  onTap: () => run(() => _rename(e)),
                ),
                ListTile(
                  leading: const Icon(Icons.drive_file_move_outlined),
                  title: const Text('Move'),
                  onTap: () => run(() => _move([e])),
                ),
                ListTile(
                  leading: Icon(
                    Icons.delete_outline,
                    color: Theme.of(ctx).colorScheme.error,
                  ),
                  title: const Text('Delete'),
                  onTap: () => run(() => _delete([e])),
                ),
                ListTile(
                  key: const Key('item-details'),
                  leading: const Icon(Icons.info_outline),
                  title: const Text('Details'),
                  onTap: () => run(
                    () => showEntryDetails(
                      context,
                      e,
                      location: _crumbs.map((c) => c.name).join(' / '),
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

  // ---- vault actions -----------------------------------------------------

  Future<void> _changePassword() async {
    final pw = await askSecret(
      context,
      title: 'Change password',
      message:
          'The old password stops working. The recovery phrase stays '
          'the same.',
      label: 'New password',
      action: 'Change',
      isNew: true,
    );
    if (pw == null || !mounted) return;
    await guarded(
      context,
      () => _s.changePassword(newPassword: pw),
      success: (_) => 'Password changed.',
    );
    pw.clear();
  }

  Future<void> _newRecovery() async {
    final ok = await confirm(
      context,
      title: 'Make a new recovery phrase?',
      message:
          'The current recovery phrase stops working as soon as the new one '
          'is made. Have paper ready to write the new one down.',
      action: 'Make new phrase',
      destructive: true,
    );
    if (!ok || !mounted) return;
    final phrase = await guarded(context, _s.newRecoveryPhrase);
    if (phrase == null || !mounted) return;
    await Navigator.of(context).push(
      MaterialPageRoute<void>(
        builder: (_) => RecoveryPhrasePage(
          phrase: phrase,
          onDone: (ctx) => Navigator.of(ctx).pop(),
        ),
      ),
    );
  }

  /// Turns fingerprint/face unlock for this vault on this device on or off.
  Future<void> _biometricSettings() async {
    final app = AppScope.read(context);
    final device = app.deviceId;
    final status = await app.biometrics.status();
    if (!mounted) return;
    if (device == null || status == BiometricStatus.unavailable) {
      await confirm(
        context,
        title: 'Fingerprint unlock',
        message:
            'This device has no strong fingerprint or face sensor that '
            'Atropos can use. On Android it needs a class 3 (strong) '
            'biometric. Windows Hello and Touch ID come in a later version.',
      );
      return;
    }
    if (status == BiometricStatus.noneEnrolled) {
      await confirm(
        context,
        title: 'Fingerprint unlock',
        message:
            'Add a fingerprint or face in the system settings first, then '
            'come back here.',
      );
      return;
    }
    final vaultId = _s.vaultId;
    final expires = await guarded(
      context,
      () => _s.deviceExpires(deviceId: device),
    );
    final on = expires != null && await app.biometrics.has(vaultId);
    if (!mounted) return;
    if (on) {
      final off = await confirm(
        context,
        title: 'Fingerprint unlock is on',
        message:
            'It works until ${_when(expires)}. Every unlock with the password '
            'or recovery phrase on this device extends it by 24 hours.\n\n'
            'Turn it off for this vault?',
        action: 'Turn off',
        destructive: true,
      );
      if (!off || !mounted) return;
      await guarded(context, () async {
        await _s.removeDevice(deviceId: device);
        await app.biometrics.remove(vaultId);
      }, success: (_) => 'Fingerprint unlock is off for this vault.');
      return;
    }
    final go = await confirm(
      context,
      title: 'Turn on fingerprint unlock?',
      message:
          'Your fingerprint or face then opens this vault on this device. '
          'The key lives in the phone\'s secure hardware and never leaves it; '
          'the vault folder only gets a slot that this device can open.\n\n'
          'For safety the password (or recovery phrase) is needed again '
          'after 24 hours, and adding a new fingerprint to the phone turns '
          'this off.',
      action: 'Turn on',
    );
    if (!go || !mounted) return;
    await guarded(context, () async {
      final secret = await _s.enrollDevice(
        deviceId: device,
        label: Platform.localHostname,
      );
      bool ok;
      try {
        ok = await app.biometrics.enroll(vaultId, secret);
      } catch (_) {
        await _s.removeDevice(deviceId: device);
        rethrow;
      } finally {
        secret.fillRange(0, secret.length, 0);
      }
      if (!ok) await _s.removeDevice(deviceId: device);
      return ok;
    }, success: (ok) => ok ? 'Fingerprint unlock is on.' : 'Cancelled.');
  }

  static String _when(int unix) {
    final t = DateTime.fromMillisecondsSinceEpoch(unix * 1000);
    String two(int n) => n.toString().padLeft(2, '0');
    return '${t.year}-${two(t.month)}-${two(t.day)} ${two(t.hour)}:${two(t.minute)}';
  }

  Future<void> _info() async {
    final t = await guarded(context, _s.totals);
    if (t == null || !mounted) return;
    final created = _s.created();
    await confirm(
      context,
      title: _s.name,
      message:
          '${plural(t.files, 'file')} in ${plural(t.folders, 'folder')}, '
          '${formatBytes(t.bytes.toInt())}.\n'
          '${created > 0 ? 'Created ${relativeTime(created)}.\n' : ''}'
          'Location: ${_s.path}\n\n'
          'Everything is encrypted with AES-256-GCM under keys wrapped by '
          'ML-KEM-1024 + ECDH P-384. File names, sizes and folder structure '
          'are hidden inside the encrypted manifest.',
    );
  }

  Future<void> _rekey() async {
    final app = AppScope.read(context);
    final oldId = _s.vaultId;
    // The old session locks during the re-key; don't treat that as a lock.
    _closing = true;
    final fresh = await rekeyFlow(context, _s);
    if (fresh == null) {
      _closing = false;
      return;
    }
    try {
      await app.biometrics.remove(oldId);
    } catch (_) {}
    await app.opened(fresh);
    if (!mounted) return;
    Navigator.of(context).pushReplacement(
      MaterialPageRoute<void>(builder: (_) => VaultPage(session: fresh)),
    );
  }

  Future<void> _lock() async {
    await AppScope.read(context).lock(_s.path);
  }

  // ---- build -------------------------------------------------------------

  static Widget _iconFor(EntryInfo e, ColorScheme scheme) =>
      Icon(switch (e.kind) {
        EntryKind.folder => Icons.folder,
        EntryKind.link => Icons.link,
        EntryKind.file => previewIcon(effectiveKind(e)),
      }, color: e.kind == EntryKind.folder ? scheme.primary : null);

  static String _subtitle(EntryInfo e) {
    final when = e.modified > 0 ? relativeTime(e.modified) : '';
    final what = switch (e.kind) {
      EntryKind.folder => e.children == 1 ? '1 item' : '${e.children} items',
      EntryKind.link => 'Link',
      EntryKind.file => formatBytes(e.size.toInt()),
    };
    return [what, if (when.isNotEmpty) when].join(' · ');
  }

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final selecting = _selected.isNotEmpty;
    final sel = _selectedEntries;
    return PopScope(
      canPop: _crumbs.isEmpty && !selecting,
      onPopInvokedWithResult: (didPop, _) {
        if (didPop) return;
        if (selecting) {
          setState(_selected.clear);
        } else {
          _goTo(_crumbs.length - 1);
        }
      },
      child: Scaffold(
        appBar: AppBar(
          leading: selecting
              ? IconButton(
                  tooltip: 'Clear selection',
                  icon: const Icon(Icons.close),
                  onPressed: () => setState(_selected.clear),
                )
              : null,
          title: Text(selecting ? '${_selected.length} selected' : _s.name),
          actions: selecting
              ? [
                  IconButton(
                    tooltip: 'Select all',
                    icon: const Icon(Icons.select_all),
                    onPressed: () => setState(
                      () => _selected.addAll(_entries.map((e) => e.id)),
                    ),
                  ),
                  IconButton(
                    key: const Key('selection-export'),
                    tooltip: 'Export unencrypted',
                    icon: const Icon(Icons.file_download_outlined),
                    onPressed: () => _export(sel),
                  ),
                  IconButton(
                    tooltip: 'Seal into a package',
                    icon: const Icon(Icons.inventory_2_outlined),
                    onPressed: () => _seal(sel),
                  ),
                  IconButton(
                    tooltip: 'Move',
                    icon: const Icon(Icons.drive_file_move_outlined),
                    onPressed: () => _move(sel),
                  ),
                  IconButton(
                    tooltip: 'Delete',
                    icon: const Icon(Icons.delete_outline),
                    onPressed: () => _delete(sel),
                  ),
                ]
              : [
                  IconButton(
                    key: const Key('vault-search'),
                    tooltip: 'Search this vault',
                    icon: const Icon(Icons.search),
                    onPressed: _search,
                  ),
                  SortButton(
                    sort: AppScope.of(context).sortFor(_s.path),
                    onChanged: _sort,
                  ),
                  IconButton(
                    key: const Key('lock-vault'),
                    tooltip: 'Lock',
                    icon: const Icon(Icons.lock_outline),
                    onPressed: _lock,
                  ),
                  PopupMenuButton<String>(
                    onSelected: (v) => switch (v) {
                      'info' => _info(),
                      'password' => _changePassword(),
                      'strength' => passwordStrengthFlow(context, _s),
                      'rekey' => _rekey(),
                      'check' =>
                        Navigator.of(context)
                            .push(
                              MaterialPageRoute<void>(
                                builder: (_) => CheckPage(session: _s),
                              ),
                            )
                            .then((_) => _reload()),
                      'recovery' => _newRecovery(),
                      'biometric' => _biometricSettings(),
                      'cards' => Navigator.of(context).push(
                        MaterialPageRoute<void>(
                          builder: (_) => CardsPage(session: _s),
                        ),
                      ),
                      'package' => openPackageIntoVault(
                        context,
                        _s,
                        _parent,
                      ).then((ok) => ok ? _reload() : null),
                      'browse' => browsePackageFlow(context, vault: _s),
                      _ => null,
                    },
                    itemBuilder: (_) => const [
                      PopupMenuItem(value: 'info', child: Text('Vault info')),
                      PopupMenuItem(
                        value: 'browse',
                        child: Text('View a package'),
                      ),
                      PopupMenuItem(
                        value: 'package',
                        child: Text('Add a package to this vault'),
                      ),
                      PopupMenuItem(
                        value: 'password',
                        child: Text('Change password'),
                      ),
                      PopupMenuItem(
                        value: 'strength',
                        child: Text('Password protection'),
                      ),
                      PopupMenuItem(
                        value: 'recovery',
                        child: Text('New recovery phrase'),
                      ),
                      PopupMenuItem(
                        value: 'biometric',
                        child: Text('Fingerprint unlock'),
                      ),
                      PopupMenuItem(
                        value: 'cards',
                        child: Text('Security cards'),
                      ),
                      PopupMenuItem(
                        value: 'check',
                        child: Text('Check and repair'),
                      ),
                      PopupMenuItem(
                        value: 'rekey',
                        child: Text('Re-key vault'),
                      ),
                    ],
                  ),
                ],
        ),
        floatingActionButton: selecting
            ? null
            : FloatingActionButton.extended(
                key: const Key('vault-add'),
                onPressed: _add,
                icon: const Icon(Icons.add),
                label: const Text('Add'),
              ),
        body: Column(
          children: [
            DamagedBanner(session: _s),
            _Breadcrumbs(
              root: _s.name,
              crumbs: _crumbs.map((c) => c.name).toList(),
              onTap: _goTo,
            ),
            const Divider(height: 1),
            Expanded(
              child: _loading
                  ? const Center(child: CircularProgressIndicator())
                  : _entries.isEmpty
                  ? _Empty(isRoot: _crumbs.isEmpty)
                  : RefreshIndicator(
                      onRefresh: _reload,
                      child: ListView.builder(
                        padding: const EdgeInsets.only(bottom: 88),
                        itemCount: _entries.length,
                        itemBuilder: (_, i) {
                          final e = _entries[i];
                          final on = _selected.contains(e.id);
                          return GestureDetector(
                            onSecondaryTap: selecting
                                ? null
                                : () => _details(e),
                            child: ListTile(
                              key: Key('entry-${e.name}'),
                              selected: on,
                              selectedTileColor: scheme.secondaryContainer
                                  .withValues(alpha: 0.5),
                              leading: selecting
                                  ? Checkbox(
                                      value: on,
                                      onChanged: (_) => _toggle(e),
                                    )
                                  : _iconFor(e, scheme),
                              title: Text(
                                e.name,
                                maxLines: 1,
                                overflow: TextOverflow.ellipsis,
                              ),
                              subtitle: Text(_subtitle(e)),
                              onTap: () => _open(e),
                              onLongPress: () =>
                                  selecting ? _toggle(e) : _details(e),
                              trailing: selecting
                                  ? null
                                  : IconButton(
                                      key: Key('more-${e.name}'),
                                      tooltip: 'More',
                                      icon: const Icon(Icons.more_vert),
                                      onPressed: () => _details(e),
                                    ),
                            ),
                          );
                        },
                      ),
                    ),
            ),
          ],
        ),
      ),
    );
  }
}

class _Breadcrumbs extends StatelessWidget {
  const _Breadcrumbs({
    required this.root,
    required this.crumbs,
    required this.onTap,
  });

  final String root;
  final List<String> crumbs;
  final ValueChanged<int> onTap;

  @override
  Widget build(BuildContext context) {
    final names = [root, ...crumbs];
    return SizedBox(
      height: 44,
      child: ListView.separated(
        scrollDirection: Axis.horizontal,
        reverse: true,
        padding: const EdgeInsets.symmetric(horizontal: 8),
        itemCount: names.length,
        separatorBuilder: (_, _) => const Icon(Icons.chevron_left, size: 18),
        itemBuilder: (_, i) {
          final depth = names.length - 1 - i;
          return Center(
            child: TextButton(
              onPressed: depth == crumbs.length ? null : () => onTap(depth),
              child: Text(names[depth]),
            ),
          );
        },
      ),
    );
  }
}

class _Empty extends StatelessWidget {
  const _Empty({required this.isRoot});
  final bool isRoot;

  @override
  Widget build(BuildContext context) {
    return Center(
      child: Padding(
        padding: const EdgeInsets.all(32),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(
              Icons.lock_outline,
              size: 48,
              color: Theme.of(context).colorScheme.outline,
            ),
            const SizedBox(height: 12),
            Text(
              isRoot
                  ? 'This vault is empty. Tap Add to encrypt files or folders '
                        'into it.'
                  : 'This folder is empty.',
              textAlign: TextAlign.center,
            ),
          ],
        ),
      ),
    );
  }
}

/// Picks a folder inside the vault as a move target. Returns a crumb with an
/// empty id for the top level.
class _FolderChooser extends StatefulWidget {
  const _FolderChooser({required this.session, required this.excluded});

  final VaultSession session;
  final Set<String> excluded;

  @override
  State<_FolderChooser> createState() => _FolderChooserState();
}

class _FolderChooserState extends State<_FolderChooser> {
  final List<_Crumb> _path = [];
  List<EntryInfo>? _folders;

  @override
  void initState() {
    super.initState();
    _load();
  }

  Future<void> _load() async {
    setState(() => _folders = null);
    final list = await widget.session.list(
      parent: _path.isEmpty ? null : _path.last.id,
    );
    if (!mounted) return;
    setState(
      () => _folders = list
          .where(
            (e) =>
                e.kind == EntryKind.folder && !widget.excluded.contains(e.id),
          )
          .toList(),
    );
  }

  @override
  Widget build(BuildContext context) {
    final here = _path.isEmpty ? _Crumb('', widget.session.name) : _path.last;
    final folders = _folders;
    return AlertDialog(
      title: Text('Move to "${here.name}"'),
      content: SizedBox(
        width: 400,
        height: 320,
        child: folders == null
            ? const Center(child: CircularProgressIndicator())
            : ListView(
                children: [
                  if (_path.isNotEmpty)
                    ListTile(
                      leading: const Icon(Icons.arrow_upward),
                      title: const Text('Up'),
                      onTap: () {
                        _path.removeLast();
                        _load();
                      },
                    ),
                  for (final f in folders)
                    ListTile(
                      leading: const Icon(Icons.folder),
                      title: Text(f.name),
                      onTap: () {
                        _path.add(_Crumb(f.id, f.name));
                        _load();
                      },
                    ),
                  if (folders.isEmpty)
                    const Padding(
                      padding: EdgeInsets.all(16),
                      child: Text('No folders here.'),
                    ),
                ],
              ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.pop(context),
          child: const Text('Cancel'),
        ),
        FilledButton(
          onPressed: () => Navigator.pop(context, here),
          child: const Text('Move here'),
        ),
      ],
    );
  }
}

/// Finds files and folders anywhere in the vault by name. Returns the hit
/// that was tapped.
class VaultSearchPage extends StatefulWidget {
  const VaultSearchPage({super.key, required this.session});
  final VaultSession session;

  @override
  State<VaultSearchPage> createState() => _VaultSearchPageState();
}

class _VaultSearchPageState extends State<VaultSearchPage> {
  List<SearchHit> _hits = const [];
  String _query = '';
  bool _busy = false;
  int _ticket = 0;

  Future<void> _search(String q) async {
    _query = q;
    final t = ++_ticket;
    if (q.trim().isEmpty) {
      setState(() => _hits = const []);
      return;
    }
    setState(() => _busy = true);
    try {
      final hits = await widget.session.search(query: q);
      if (mounted && t == _ticket) setState(() => _hits = hits);
    } catch (e) {
      if (mounted) showMessage(context, e.toString(), error: true);
    } finally {
      if (mounted && t == _ticket) setState(() => _busy = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return Scaffold(
      appBar: AppBar(
        title: TextField(
          key: const Key('vault-search-field'),
          autofocus: true,
          autocorrect: false,
          enableSuggestions: false,
          enableIMEPersonalizedLearning: false,
          decoration: InputDecoration(
            hintText: 'Search ${widget.session.name} by name',
            border: InputBorder.none,
          ),
          onChanged: _search,
        ),
        bottom: _busy
            ? const PreferredSize(
                preferredSize: Size.fromHeight(2),
                child: LinearProgressIndicator(minHeight: 2),
              )
            : null,
      ),
      body: _query.trim().isEmpty
          ? const Center(
              child: Padding(
                padding: EdgeInsets.all(24),
                child: Text(
                  'Type part of a file or folder name. The search runs on '
                  'the encrypted list of names in memory; nothing is '
                  'decrypted to disk.',
                  textAlign: TextAlign.center,
                ),
              ),
            )
          : _hits.isEmpty && !_busy
          ? const Center(child: Text('Nothing found.'))
          : ListView.builder(
              key: const Key('vault-search-results'),
              itemCount: _hits.length,
              itemBuilder: (context, i) {
                final h = _hits[i];
                final where = h.folders.isEmpty
                    ? widget.session.name
                    : h.folders.map((f) => f.name).join(' / ');
                return ListTile(
                  key: Key('hit-${h.entry.name}'),
                  leading: _VaultPageState._iconFor(h.entry, scheme),
                  title: Text(h.entry.name, overflow: TextOverflow.ellipsis),
                  subtitle: Text(
                    'In $where · ${_VaultPageState._subtitle(h.entry)}',
                    maxLines: 2,
                    overflow: TextOverflow.ellipsis,
                  ),
                  onTap: () => Navigator.pop(context, h),
                );
              },
            ),
    );
  }
}
