import 'package:flutter/material.dart';

import '../core/crypto_service.dart';
import '../core/errors.dart';

/// The on-device cryptography self-test, reachable from settings.
class SelfTestPage extends StatelessWidget {
  const SelfTestPage({super.key, required this.crypto});
  final CryptoService crypto;

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(title: const Text('Cryptography self-test')),
      body: ListView(
        padding: const EdgeInsets.all(16),
        children: [SelfTestCard(crypto: crypto)],
      ),
    );
  }
}

/// Runs the Rust self-test and lists each check with its timing.
class SelfTestCard extends StatefulWidget {
  const SelfTestCard({super.key, required this.crypto});
  final CryptoService crypto;

  @override
  State<SelfTestCard> createState() => _SelfTestCardState();
}

class _SelfTestCardState extends State<SelfTestCard> {
  bool _running = false;
  SelfTestReport? _report;
  String? _error;

  Future<void> _run() async {
    setState(() {
      _running = true;
      _error = null;
    });
    try {
      final r = await widget.crypto.runSelfTest();
      if (mounted) setState(() => _report = r);
    } catch (e) {
      if (mounted) setState(() => _error = describeError(e));
    } finally {
      if (mounted) setState(() => _running = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final r = _report;
    return Card(
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(
              'Cryptography self-test',
              style: Theme.of(context).textTheme.titleMedium,
            ),
            const SizedBox(height: 4),
            const Text(
              'Runs every primitive once on this device. The Argon2id step '
              'uses 256 MiB of memory and shows how long one unlock will take.',
            ),
            const SizedBox(height: 12),
            FilledButton.icon(
              key: const Key('run-self-test'),
              onPressed: _running ? null : _run,
              icon: _running
                  ? const SizedBox.square(
                      dimension: 16,
                      child: CircularProgressIndicator(strokeWidth: 2),
                    )
                  : const Icon(Icons.play_arrow),
              label: Text(_running ? 'Running…' : 'Run self-test'),
            ),
            if (_error != null) ...[
              const SizedBox(height: 12),
              Text(_error!, style: TextStyle(color: scheme.error)),
            ],
            if (r != null) ...[
              const SizedBox(height: 12),
              Text(
                r.passed ? 'All checks passed' : 'Some checks failed',
                key: const Key('self-test-result'),
                style: TextStyle(
                  fontWeight: FontWeight.w600,
                  color: r.passed ? scheme.primary : scheme.error,
                ),
              ),
              for (final s in r.steps)
                ListTile(
                  contentPadding: EdgeInsets.zero,
                  leading: Icon(
                    s.passed ? Icons.check_circle : Icons.error,
                    color: s.passed ? scheme.primary : scheme.error,
                  ),
                  title: Text(s.name),
                  subtitle: Text('${s.detail} · ${s.millis} ms'),
                ),
            ],
          ],
        ),
      ),
    );
  }
}
