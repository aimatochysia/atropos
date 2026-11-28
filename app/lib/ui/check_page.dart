import 'dart:async';

import 'package:flutter/material.dart';

import '../core/errors.dart';
import '../core/format.dart';
import '../src/rust/api/maintenance.dart';
import '../src/rust/api/secure_input.dart';
import '../src/rust/api/vault.dart';
import 'recovery_page.dart';
import 'secret_dialog.dart';
import 'task_progress.dart';
import 'widgets/feedback.dart';

/// Decrypts everything in the vault, reports problems and offers a repair.
class CheckPage extends StatefulWidget {
  const CheckPage({super.key, required this.session});
  final VaultSession session;

  @override
  State<CheckPage> createState() => _CheckPageState();
}

class _CheckPageState extends State<CheckPage> {
  StreamSubscription<CheckEvent>? _sub;
  ProgressInfo? _progress;
  CheckReportInfo? _report;
  RepairInfo? _repaired;
  String? _error;
  bool _removeBroken = false;
  bool _repairing = false;

  @override
  void initState() {
    super.initState();
    _start();
  }

  void _start() {
    _sub?.cancel();
    setState(() {
      _progress = null;
      _report = null;
      _error = null;
      _removeBroken = false;
    });
    _sub = widget.session.check().listen(
      (e) => setState(() {
        switch (e) {
          case CheckEvent_Progress(:final field0):
            _progress = field0;
          case CheckEvent_Done(:final field0):
            _report = field0;
        }
      }),
      onError: (Object e) => setState(() => _error = describeError(e)),
    );
  }

  @override
  void dispose() {
    _sub?.cancel();
    super.dispose();
  }

  Future<void> _repair() async {
    final r = _report;
    if (r == null) return;
    final broken = r.damaged.length + r.missing.length;
    if (_removeBroken && broken > 0) {
      final ok = await confirm(
        context,
        title: 'Remove ${plural(broken, 'file')}?',
        message:
            'They are taken off the list in this vault for good. If a file '
            'is only missing because another device has not finished '
            'syncing, wait for the sync and check again instead.',
        action: 'Remove',
        destructive: true,
      );
      if (!ok || !mounted) return;
    }
    setState(() => _repairing = true);
    try {
      final done = await widget.session.repair(removeBroken: _removeBroken);
      if (!mounted) return;
      setState(() {
        _repaired = done;
        _repairing = false;
      });
      _start();
    } catch (e) {
      if (!mounted) return;
      setState(() => _repairing = false);
      showMessage(context, describeError(e), error: true);
    }
  }

  String _repairSummary(RepairInfo r) {
    final parts = [
      if (r.removedEntries > 0) 'removed ${plural(r.removedEntries, 'file')}',
      if (r.movedToTop > 0) 'moved ${plural(r.movedToTop, 'item')} to the top',
      if (r.deletedOrphans > 0)
        'deleted ${plural(r.deletedOrphans, 'unused piece')} '
            '(${formatBytes(r.freedBytes.toInt())})',
      if (r.deletedLeftovers > 0)
        'cleaned up ${plural(r.deletedLeftovers, 'temporary file')}',
      if (r.segmentsSetAside > 0)
        'set aside ${plural(r.segmentsSetAside, 'damaged part')} of the file '
            'list in the "damaged" folder',
    ];
    if (parts.isEmpty) return 'Nothing needed fixing.';
    final s = parts.join(', ');
    return '${s[0].toUpperCase()}${s.substring(1)}.';
  }

  Widget _section(String title, String help, List<String> items) {
    if (items.isEmpty) return const SizedBox.shrink();
    return Card(
      child: ExpansionTile(
        title: Text('$title (${items.length})'),
        subtitle: Text(help),
        children: [
          for (final p in items.take(200))
            ListTile(dense: true, title: Text(p)),
          if (items.length > 200)
            ListTile(
              dense: true,
              title: Text('…and ${items.length - 200} more'),
            ),
        ],
      ),
    );
  }

  Widget _results(CheckReportInfo r) {
    final scheme = Theme.of(context).colorScheme;
    final t = Theme.of(context).textTheme;
    final broken = r.damaged.length + r.missing.length;
    final fixable =
        r.orphans > 0 ||
        r.leftovers > 0 ||
        r.lost.isNotEmpty ||
        r.damagedSegments > 0 ||
        broken > 0;
    return ListView(
      key: const Key('check-results'),
      padding: const EdgeInsets.all(16),
      children: [
        if (_repaired != null)
          Card(
            color: scheme.secondaryContainer,
            child: Padding(
              padding: const EdgeInsets.all(16),
              child: Text(_repairSummary(_repaired!)),
            ),
          ),
        Icon(
          r.clean ? Icons.verified_outlined : Icons.report_problem_outlined,
          size: 56,
          color: r.clean ? scheme.primary : scheme.error,
        ),
        const SizedBox(height: 8),
        Text(
          r.clean ? 'Everything checks out' : 'Problems found',
          key: Key(r.clean ? 'check-clean' : 'check-problems'),
          textAlign: TextAlign.center,
          style: t.titleLarge,
        ),
        const SizedBox(height: 4),
        Text(
          '${plural(r.filesChecked.toInt(), 'file')} '
          '(${formatBytes(r.bytesChecked.toInt())}) decrypted and verified.',
          textAlign: TextAlign.center,
        ),
        const SizedBox(height: 16),
        _section(
          'Damaged files',
          'Their encrypted data was changed or is corrupt.',
          r.damaged,
        ),
        _section(
          'Missing files',
          'Their data is not in this copy. If you sync this vault, another '
              'device may not have finished uploading.',
          r.missing,
        ),
        _section(
          'Items without a folder',
          'Their folder is gone. Repair moves them to the top level.',
          r.lost,
        ),
        if (r.damagedSegments > 0)
          Card(
            child: ListTile(
              leading: const Icon(Icons.list_alt),
              title: Text(
                '${plural(r.damagedSegments, 'part')} of the file list '
                'could not be read',
              ),
              subtitle: const Text(
                'What it described may be missing from the list. Repair sets '
                'it aside in the "damaged" folder instead of deleting it.',
              ),
            ),
          ),
        if (r.orphans > 0 || r.leftovers > 0)
          Card(
            child: ListTile(
              leading: const Icon(Icons.cleaning_services_outlined),
              title: Text(
                [
                  if (r.orphans > 0)
                    '${plural(r.orphans, 'unused piece')} '
                        '(${formatBytes(r.orphanBytes.toInt())})',
                  if (r.leftovers > 0) plural(r.leftovers, 'temporary file'),
                ].join(' and '),
              ),
              subtitle: const Text(
                'Left by an interrupted import or save. Safe to delete.',
              ),
            ),
          ),
        if (broken > 0)
          CheckboxListTile(
            key: const Key('check-remove-broken'),
            value: _removeBroken,
            onChanged: (v) => setState(() => _removeBroken = v ?? false),
            title: Text(
              'Also remove the ${plural(broken, 'damaged or missing file')} '
              'from the list',
            ),
          ),
        const SizedBox(height: 8),
        if (fixable)
          FilledButton.icon(
            key: const Key('check-repair'),
            onPressed: _repairing ? null : _repair,
            icon: const Icon(Icons.build_outlined),
            label: Text(_repairing ? 'Repairing…' : 'Repair'),
          ),
        TextButton(onPressed: _start, child: const Text('Check again')),
      ],
    );
  }

  @override
  Widget build(BuildContext context) {
    final r = _report;
    final p = _progress;
    return Scaffold(
      appBar: AppBar(title: const Text('Check vault')),
      body: _error != null
          ? Center(
              child: Padding(
                padding: const EdgeInsets.all(24),
                child: Text(_error!, textAlign: TextAlign.center),
              ),
            )
          : r != null
          ? _results(r)
          : Padding(
              padding: const EdgeInsets.all(24),
              child: Column(
                mainAxisAlignment: MainAxisAlignment.center,
                children: [
                  LinearProgressIndicator(
                    value: p == null || p.bytesTotal == BigInt.zero
                        ? null
                        : p.bytesDone.toDouble() / p.bytesTotal.toDouble(),
                  ),
                  const SizedBox(height: 12),
                  Text(
                    p == null
                        ? 'Starting…'
                        : 'Checked ${p.filesDone} of ${plural(p.filesTotal.toInt(), 'file')}',
                  ),
                  if (p != null && p.current.isNotEmpty)
                    Text(
                      p.current,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: Theme.of(context).textTheme.bodySmall,
                    ),
                ],
              ),
            ),
    );
  }
}

/// Lets the person pick how much work each password guess costs.
Future<void> passwordStrengthFlow(
  BuildContext context,
  VaultSession session,
) async {
  final current = await guarded(context, session.passwordStrength);
  if (current == null || !context.mounted) return;
  final picked = await showDialog<PasswordStrength>(
    context: context,
    builder: (ctx) => _StrengthDialog(current: current),
  );
  if (picked == null || picked == current || !context.mounted) return;
  final pw = await askSecret(
    context,
    title: 'Enter the password',
    message: 'The password stays the same; only how it is protected changes.',
    label: 'Password',
    action: 'Apply',
  );
  if (pw == null || !context.mounted) return;
  await _applyStrength(context, session, pw, picked);
  pw.clear();
}

Future<void> _applyStrength(
  BuildContext context,
  VaultSession session,
  SecretInput pw,
  PasswordStrength s,
) async {
  showDialog<void>(
    context: context,
    barrierDismissible: false,
    builder: (_) => const AlertDialog(
      content: Row(
        children: [
          CircularProgressIndicator(),
          SizedBox(width: 16),
          Expanded(child: Text('Measuring this device and saving…')),
        ],
      ),
    ),
  );
  try {
    final ms = await session.setPasswordStrength(password: pw, strength: s);
    if (!context.mounted) return;
    Navigator.of(context).pop();
    showMessage(
      context,
      'Saved. Unlocking with the password now takes about '
      '${(ms / 1000).toStringAsFixed(1)} s on this device.',
    );
  } catch (e) {
    if (!context.mounted) return;
    Navigator.of(context).pop();
    showMessage(context, describeError(e), error: true);
  }
}

class _StrengthDialog extends StatefulWidget {
  const _StrengthDialog({required this.current});
  final PasswordStrength current;

  @override
  State<_StrengthDialog> createState() => _StrengthDialogState();
}

class _StrengthDialogState extends State<_StrengthDialog> {
  late PasswordStrength _v = widget.current == PasswordStrength.custom
      ? PasswordStrength.standard
      : widget.current;

  @override
  Widget build(BuildContext context) {
    Widget option(PasswordStrength s, String title, String sub) =>
        RadioListTile<PasswordStrength>(
          key: Key('strength-${s.name}'),
          value: s,
          title: Text(title),
          subtitle: Text(sub),
        );
    return AlertDialog(
      title: const Text('Password protection'),
      content: SingleChildScrollView(
        child: RadioGroup<PasswordStrength>(
          groupValue: _v,
          onChanged: (v) => setState(() => _v = v ?? _v),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              const Text(
                'Each guess at the password costs an attacker this much '
                'memory and time. Higher is safer but unlocks slower, and '
                'needs a device with that much free memory.',
              ),
              if (widget.current == PasswordStrength.custom)
                const Padding(
                  padding: EdgeInsets.only(top: 8),
                  child: Text('Now: a custom setting.'),
                ),
              option(
                PasswordStrength.light,
                'Light (64 MB)',
                'For older phones with little memory.',
              ),
              option(
                PasswordStrength.standard,
                'Standard (256 MB)',
                'The default.',
              ),
              option(
                PasswordStrength.strong,
                'Strong (1 GB)',
                'About four times the work. A device with less than about '
                    '2 GB of memory may not open this vault with the '
                    'password; the recovery phrase still works.',
              ),
            ],
          ),
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.pop(context),
          child: const Text('Cancel'),
        ),
        FilledButton(
          key: const Key('strength-save'),
          onPressed: () => Navigator.pop(context, _v),
          child: const Text('Continue'),
        ),
      ],
    );
  }
}

/// A one-line warning shown on the vault page when part of its file list
/// could not be read.
class DamagedBanner extends StatelessWidget {
  const DamagedBanner({super.key, required this.session});
  final VaultSession session;

  @override
  Widget build(BuildContext context) {
    final n = session.damagedParts();
    if (n == 0) return const SizedBox.shrink();
    final scheme = Theme.of(context).colorScheme;
    return MaterialBanner(
      key: const Key('damaged-banner'),
      backgroundColor: scheme.errorContainer,
      content: const Text(
        'Part of this vault\'s file list could not be read, so some items '
        'may not be shown.',
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).push(
            MaterialPageRoute<void>(
              builder: (_) => CheckPage(session: session),
            ),
          ),
          child: const Text('Check and repair'),
        ),
      ],
    );
  }
}

/// Re-encrypts the vault under all-new keys. Returns the new session (the
/// old one is locked), or null when cancelled or failed.
Future<VaultSession?> rekeyFlow(BuildContext context, VaultSession s) async {
  final ok = await confirm(
    context,
    title: 'Re-key this vault?',
    message:
        'Atropos makes all-new keys and a new recovery phrase, and encrypts '
        'every file again. Use it if you think a key or the recovery phrase '
        'leaked.\n\n'
        '• The password stays the same.\n'
        '• The old recovery phrase stops working for this copy. Have paper '
        'ready for the new one.\n'
        '• Fingerprint and security card unlock must be set up again.\n'
        '• Copies made before (backups, cloud history, git) still open with '
        'the old password or phrase.\n'
        '• It takes about as long as importing everything again, and needs '
        'that much free space while it runs.',
    action: 'Continue',
    destructive: true,
  );
  if (!ok || !context.mounted) return null;
  final pw = await askSecret(
    context,
    title: 'Enter the password',
    label: 'Password',
    action: 'Re-key',
  );
  if (pw == null || !context.mounted) return null;
  final done = await showDialog<RekeyDone>(
    context: context,
    barrierDismissible: false,
    builder: (_) => KeysProgressDialog(
      title: 'Re-keying',
      start: () => s.rekey(password: pw),
    ),
  );
  pw.clear();
  if (done == null) return null;
  if (context.mounted) {
    await Navigator.of(context).push(
      MaterialPageRoute<void>(
        builder: (_) => RecoveryPhrasePage(
          phrase: done.recoveryPhrase,
          onDone: (ctx) => Navigator.of(ctx).pop(),
        ),
      ),
    );
  }
  return done.session;
}

/// Shows a re-key or upgrade running; pops with its result.
class KeysProgressDialog extends StatefulWidget {
  const KeysProgressDialog({
    super.key,
    required this.title,
    required this.start,
  });
  final String title;
  final Stream<RekeyEvent> Function() start;

  @override
  State<KeysProgressDialog> createState() => _KeysProgressDialogState();
}

class _KeysProgressDialogState extends State<KeysProgressDialog> {
  StreamSubscription<RekeyEvent>? _sub;
  ProgressInfo? _p;
  String? _error;

  @override
  void initState() {
    super.initState();
    _sub = widget.start().listen((e) {
      if (!mounted) return;
      final done = e.done();
      if (done != null) {
        Navigator.of(context).pop(done);
        return;
      }
      setState(() => _p = e.progress());
    }, onError: (Object e) => setState(() => _error = describeError(e)));
  }

  @override
  void dispose() {
    _sub?.cancel();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final p = _p;
    return AlertDialog(
      title: Text(widget.title),
      content: _error != null
          ? Text(_error!)
          : Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                LinearProgressIndicator(
                  value: p == null || p.bytesTotal == BigInt.zero
                      ? null
                      : p.bytesDone.toDouble() / p.bytesTotal.toDouble(),
                ),
                const SizedBox(height: 12),
                Text(
                  p == null
                      ? 'Making new keys…'
                      : p.filesDone == p.filesTotal
                      ? 'Verifying and switching over…'
                      : '${p.filesDone} of ${plural(p.filesTotal, 'file')}',
                ),
                const SizedBox(height: 8),
                const Text('Keep Atropos open until this finishes.'),
              ],
            ),
      actions: [
        if (_error != null)
          TextButton(
            onPressed: () => Navigator.of(context).pop(),
            child: const Text('Close'),
          ),
      ],
    );
  }
}
