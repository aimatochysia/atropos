import 'package:flutter/material.dart';

/// Shows a new 24-word recovery phrase once. It is never stored, so the
/// person has to confirm they wrote it down.
class RecoveryPhrasePage extends StatefulWidget {
  const RecoveryPhrasePage({
    super.key,
    required this.phrase,
    required this.onDone,
  });

  final String phrase;
  final void Function(BuildContext context) onDone;

  @override
  State<RecoveryPhrasePage> createState() => _RecoveryPhrasePageState();
}

class _RecoveryPhrasePageState extends State<RecoveryPhrasePage> {
  bool _written = false;

  @override
  Widget build(BuildContext context) {
    final words = widget.phrase.split(' ');
    final scheme = Theme.of(context).colorScheme;
    return PopScope(
      canPop: false,
      child: Scaffold(
        appBar: AppBar(
          title: const Text('Recovery phrase'),
          automaticallyImplyLeading: false,
        ),
        body: ListView(
          padding: const EdgeInsets.all(16),
          children: [
            const Text(
              'Write these 24 words on paper, in order, and keep them '
              'somewhere safe. They open the vault if you forget the '
              'password. Atropos will not show them again.',
            ),
            const SizedBox(height: 8),
            Text(
              'Do not photograph them or store them in a notes app.',
              style: TextStyle(color: scheme.error),
            ),
            const SizedBox(height: 16),
            Card(
              child: Padding(
                padding: const EdgeInsets.all(12),
                child: LayoutBuilder(
                  builder: (context, box) {
                    final cols = box.maxWidth > 520 ? 4 : 3;
                    return Wrap(
                      children: [
                        for (var i = 0; i < words.length; i++)
                          SizedBox(
                            width: box.maxWidth / cols,
                            child: Padding(
                              padding: const EdgeInsets.symmetric(vertical: 6),
                              child: Text.rich(
                                TextSpan(
                                  children: [
                                    TextSpan(
                                      text: '${i + 1}. ',
                                      style: TextStyle(color: scheme.outline),
                                    ),
                                    TextSpan(
                                      text: words[i],
                                      style: const TextStyle(
                                        fontFamily: 'monospace',
                                        fontWeight: FontWeight.w600,
                                      ),
                                    ),
                                  ],
                                ),
                                key: Key('recovery-word-$i'),
                              ),
                            ),
                          ),
                      ],
                    );
                  },
                ),
              ),
            ),
            const SizedBox(height: 8),
            CheckboxListTile(
              key: const Key('recovery-written'),
              contentPadding: EdgeInsets.zero,
              value: _written,
              onChanged: (v) => setState(() => _written = v ?? false),
              title: const Text('I wrote down all 24 words'),
            ),
            const SizedBox(height: 8),
            FilledButton(
              key: const Key('recovery-continue'),
              onPressed: _written ? () => widget.onDone(context) : null,
              child: const Text('Continue'),
            ),
          ],
        ),
      ),
    );
  }
}
