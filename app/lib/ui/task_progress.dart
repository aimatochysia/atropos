import 'dart:async';

import 'package:flutter/material.dart';

import '../core/errors.dart';
import '../core/format.dart';
import '../src/rust/api/vault.dart';

/// Runs an import, export or package task in a dialog that shows its
/// progress, then a summary. Returns the outcome, or null when it failed
/// (the error is shown in the dialog).
Future<OutcomeInfo?> runTask(
  BuildContext context, {
  required String title,
  required Stream<TaskEvent> Function() start,
  required String Function(OutcomeInfo) summary,
}) {
  return showDialog<OutcomeInfo>(
    context: context,
    barrierDismissible: false,
    builder: (_) => _TaskDialog(title: title, start: start, summary: summary),
  );
}

class _TaskDialog extends StatefulWidget {
  const _TaskDialog({
    required this.title,
    required this.start,
    required this.summary,
  });

  final String title;
  final Stream<TaskEvent> Function() start;
  final String Function(OutcomeInfo) summary;

  @override
  State<_TaskDialog> createState() => _TaskDialogState();
}

class _TaskDialogState extends State<_TaskDialog> {
  StreamSubscription<TaskEvent>? _sub;
  ProgressInfo? _progress;
  OutcomeInfo? _outcome;
  String? _error;

  @override
  void initState() {
    super.initState();
    _sub = widget.start().listen(
      (e) => setState(() {
        switch (e) {
          case TaskEvent_Progress(:final field0):
            _progress = field0;
          case TaskEvent_Done(:final field0):
            _outcome = field0;
        }
      }),
      onError: (Object e) => setState(() => _error = describeError(e)),
      onDone: () {
        if (mounted && _outcome == null && _error == null) {
          setState(() => _error = 'The task stopped without finishing.');
        }
      },
    );
  }

  @override
  void dispose() {
    _sub?.cancel();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final p = _progress;
    final o = _outcome;
    final scheme = Theme.of(context).colorScheme;
    final finished = o != null || _error != null;
    double? fraction;
    if (p != null && p.bytesTotal > BigInt.zero) {
      fraction = (p.bytesDone.toDouble() / p.bytesTotal.toDouble()).clamp(
        0.0,
        1.0,
      );
    }
    return PopScope(
      canPop: finished,
      child: AlertDialog(
        title: Text(widget.title),
        content: SizedBox(
          width: 420,
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              if (_error != null)
                Text(
                  _error!,
                  key: const Key('task-error'),
                  style: TextStyle(color: scheme.error),
                )
              else if (o != null) ...[
                Text(widget.summary(o), key: const Key('task-summary')),
                if (o.skipped.isNotEmpty) ...[
                  const SizedBox(height: 8),
                  Text(
                    'Skipped ${o.skipped.length}:',
                    style: const TextStyle(fontWeight: FontWeight.w600),
                  ),
                  for (final s in o.skipped.take(8))
                    Text('• $s', style: Theme.of(context).textTheme.bodySmall),
                ],
              ] else ...[
                LinearProgressIndicator(value: fraction),
                const SizedBox(height: 12),
                Text(
                  p == null
                      ? 'Starting…'
                      : '${p.filesDone} of ${p.filesTotal} files · '
                            '${formatBytes(p.bytesDone.toInt())} of '
                            '${formatBytes(p.bytesTotal.toInt())}',
                ),
                if (p != null && p.current.isNotEmpty)
                  Text(
                    p.current,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: Theme.of(context).textTheme.bodySmall,
                  ),
              ],
            ],
          ),
        ),
        actions: [
          if (finished)
            FilledButton(
              key: const Key('task-close'),
              onPressed: () => Navigator.pop(context, o),
              child: const Text('Close'),
            ),
        ],
      ),
    );
  }
}

/// "1 file", "3 files". [n] is an int or a BigInt.
String plural(Object n, String one, [String? many]) =>
    '$n ${n == 1 || n == BigInt.one ? one : (many ?? '${one}s')}';
