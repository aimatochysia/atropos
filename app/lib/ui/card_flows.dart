import 'dart:async';
import 'dart:typed_data';

import 'package:flutter/material.dart';

import '../core/cards.dart';
import '../core/errors.dart';
import '../core/format.dart';
import '../src/rust/api/card.dart' as core;
import '../src/rust/api/vault.dart' show VaultSession;
import '../state/app_controller.dart';
import 'secret_dialog.dart';
import 'widgets/feedback.dart';

typedef Transmit = FutureOr<Uint8List> Function(Uint8List apdu);

/// Shows "insert / hold your card", connects, runs [run] with the card and
/// closes it again. Returns the result, or null when cancelled or failed (the
/// error is shown).
Future<T?> withCard<T>(
  BuildContext context, {
  required String title,
  required Future<T> Function(Transmit transmit) run,
}) async {
  final access = AppScope.read(context).cards;
  final problem = await access.problem();
  if (!context.mounted) return null;
  if (problem != null) {
    showMessage(context, problem, error: true);
    return null;
  }
  return showDialog<T>(
    context: context,
    barrierDismissible: false,
    builder: (_) => _CardDialog<T>(title: title, access: access, run: run),
  );
}

class _CardDialog<T> extends StatefulWidget {
  const _CardDialog({
    required this.title,
    required this.access,
    required this.run,
  });

  final String title;
  final CardAccess access;
  final Future<T> Function(Transmit transmit) run;

  @override
  State<_CardDialog<T>> createState() => _CardDialogState<T>();
}

class _CardDialogState<T> extends State<_CardDialog<T>> {
  String? _error;
  bool _talking = false;
  bool _done = false;

  @override
  void initState() {
    super.initState();
    _go();
  }

  Future<void> _go() async {
    setState(() {
      _error = null;
      _talking = false;
    });
    CardLink? link;
    try {
      link = await widget.access.connect();
      if (link == null) {
        if (mounted && !_done) Navigator.pop(context);
        return;
      }
      if (!mounted) return;
      setState(() => _talking = true);
      final r = await widget.run(link.transmit);
      _done = true;
      if (mounted) Navigator.pop(context, r);
    } catch (e) {
      if (mounted) setState(() => _error = describeError(e));
    } finally {
      await link?.close();
    }
  }

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final nfc = widget.access.contactless;
    return AlertDialog(
      title: Text(widget.title),
      content: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          Icon(
            nfc ? Icons.contactless_outlined : Icons.credit_card,
            size: 56,
            color: _error == null ? scheme.primary : scheme.error,
          ),
          const SizedBox(height: 12),
          if (_error != null)
            Text(
              _error!,
              key: const Key('card-error'),
              textAlign: TextAlign.center,
              style: TextStyle(color: scheme.error),
            )
          else if (_talking) ...[
            const LinearProgressIndicator(),
            const SizedBox(height: 8),
            Text(nfc ? 'Keep holding the card still…' : 'Talking to the card…'),
          ] else
            Text(
              nfc
                  ? 'Hold your security card flat against the back of the '
                        'phone.'
                  : 'Insert your security card into the reader (or put it '
                        'on a USB NFC reader).',
              textAlign: TextAlign.center,
            ),
        ],
      ),
      actions: [
        TextButton(
          onPressed: () async {
            _done = true;
            await widget.access.cancel();
            if (context.mounted) Navigator.pop(context);
          },
          child: const Text('Cancel'),
        ),
        if (_error != null)
          FilledButton(onPressed: _go, child: const Text('Try again')),
      ],
    );
  }
}

/// Unlock page: card PIN, then the card.
Future<VaultSession?> unlockWithCardFlow(
  BuildContext context, {
  required String path,
  required String name,
}) async {
  final device = AppScope.read(context).deviceId;
  final pin = await askSecret(
    context,
    title: 'Security card PIN',
    message: 'Enter the PIN of the card you added to "$name".',
    label: 'Card PIN',
    action: 'Next',
  );
  if (pin == null || !context.mounted) return null;
  try {
    return await withCard(
      context,
      title: 'Unlock with card',
      run: (t) => core.unlockWithCard(
        path: path,
        pin: pin,
        deviceId: device,
        transmit: t,
      ),
    );
  } finally {
    pin.clear();
  }
}

/// Vault menu: the cards that open this vault.
class CardsPage extends StatefulWidget {
  const CardsPage({super.key, required this.session});
  final VaultSession session;

  @override
  State<CardsPage> createState() => _CardsPageState();
}

class _CardsPageState extends State<CardsPage> {
  List<core.CardSlotInfo>? _cards;

  @override
  void initState() {
    super.initState();
    _load();
  }

  Future<void> _load() async {
    final c = await widget.session.cards();
    if (mounted) setState(() => _cards = c);
  }

  Future<void> _add() async {
    // Look at the card first so a card without a key can be set up.
    final info = await withCard(
      context,
      title: 'Add a security card',
      run: (t) => core.cardStatus(transmit: t),
    );
    if (info == null || !mounted) return;
    if (!info.hasKey) {
      if (info.applet != 'openpgp') {
        await confirm(
          context,
          title: 'No key on this card',
          message:
              '${info.label} has no P-384 or P-256 key in PIV slot 9D (key '
              'management). Make one with your card\'s own tool, for '
              'example YubiKey Manager: ykman piv keys generate -a '
              'ECCP384 9d, then add the card again.',
        );
        return;
      }
      final setUp = await confirm(
        context,
        title: 'Set up this card?',
        message:
            '${info.label} has no encryption key Atropos can use. Atropos can '
            'make a new NIST P-384 key on the card. This replaces the card\'s '
            'current OpenPGP encryption key, if any: files encrypted to that '
            'key with GnuPG could no longer be decrypted.\n\n'
            'You need the card\'s admin PIN (factory default 12345678).',
        action: 'Make a key',
        destructive: true,
      );
      if (!setUp || !mounted) return;
      final admin = await askSecret(
        context,
        title: 'Admin PIN',
        label: 'Card admin PIN',
        action: 'Next',
      );
      if (admin == null || !mounted) return;
      final made = await withCard(
        context,
        title: 'Making a key on the card',
        run: (t) => core.cardGenerateKey(adminPin: admin, transmit: t),
      );
      admin.clear();
      if (made == null || !mounted) return;
    }
    final pin = await askSecret(
      context,
      title: 'Card PIN',
      message:
          'Atropos checks the card can open the vault before saving it. '
          '${info.pinTries != null ? '${info.pinTries} PIN tries left. ' : ''}'
          'Factory default PINs are 123456.',
      label: 'Card PIN',
      action: 'Next',
    );
    if (pin == null || !mounted) return;
    final label = await withCard(
      context,
      title: 'Add a security card',
      run: (t) => widget.session.addCard(pin: pin, name: '', transmit: t),
    );
    pin.clear();
    if (label == null || !mounted) return;
    showMessage(context, '$label now opens this vault.');
    _load();
  }

  Future<void> _remove(core.CardSlotInfo c) async {
    final ok = await confirm(
      context,
      title: 'Remove "${c.label}"?',
      message:
          'The card stops opening this vault. The card itself is not '
          'changed.',
      action: 'Remove',
      destructive: true,
    );
    if (!ok || !mounted) return;
    await guarded(context, () => widget.session.removeCard(id: c.id));
    _load();
  }

  @override
  Widget build(BuildContext context) {
    final cards = _cards;
    return Scaffold(
      appBar: AppBar(title: const Text('Security cards')),
      floatingActionButton: FloatingActionButton.extended(
        key: const Key('add-card'),
        onPressed: _add,
        icon: const Icon(Icons.add),
        label: const Text('Add card'),
      ),
      body: cards == null
          ? const Center(child: CircularProgressIndicator())
          : ListView(
              padding: const EdgeInsets.only(bottom: 88),
              children: [
                const Padding(
                  padding: EdgeInsets.all(16),
                  child: Text(
                    'An OpenPGP or PIV smartcard (YubiKey, Nitrokey and many '
                    'others) can open this vault with its PIN, by NFC or in a '
                    'reader. The card\'s private key never leaves the card. '
                    'A card unlock also renews fingerprint unlock for 24 '
                    'hours.',
                  ),
                ),
                if (cards.isEmpty)
                  const ListTile(
                    leading: Icon(Icons.credit_card_off_outlined),
                    title: Text('No cards yet'),
                  ),
                for (final c in cards)
                  ListTile(
                    leading: const Icon(Icons.credit_card),
                    title: Text(c.label),
                    subtitle: Text(
                      '${c.applet == 'piv' ? 'PIV' : 'OpenPGP'}'
                      '${c.added > 0 ? ' · added ${relativeTime(c.added)}' : ''}',
                    ),
                    trailing: IconButton(
                      tooltip: 'Remove',
                      icon: const Icon(Icons.delete_outline),
                      onPressed: () => _remove(c),
                    ),
                  ),
              ],
            ),
    );
  }
}
