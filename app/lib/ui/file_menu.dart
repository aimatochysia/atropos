import 'package:flutter/material.dart';

import '../core/format.dart';
import '../src/rust/api/vault.dart';
import '../src/rust/api/viewer.dart';
import 'viewer_page.dart' show previewIcon;

/// What a file looks like in the viewer: the user's "Open as" choice, or
/// what its name says.
PreviewKind effectiveKind(EntryInfo e, {PreviewKind? rule}) =>
    e.openAs ?? rule ?? previewForName(name: e.name);

String previewLabel(PreviewKind k) => switch (k) {
  PreviewKind.text => 'Text',
  PreviewKind.image => 'Picture',
  PreviewKind.pdf => 'PDF',
  PreviewKind.markdown => 'Markdown',
  PreviewKind.audio => 'Audio',
  PreviewKind.none => 'Automatic (by file type)',
};

/// Asks how to open [name]. Returns [PreviewKind.none] for "automatic",
/// or null when cancelled.
Future<PreviewKind?> pickOpenAs(
  BuildContext context, {
  required String name,
  PreviewKind? current,
  String? note,
}) => showDialog<PreviewKind>(
  context: context,
  builder: (ctx) => SimpleDialog(
    title: Text('Open "$name" as'),
    children: [
      if (note != null)
        Padding(
          padding: const EdgeInsets.fromLTRB(24, 0, 24, 8),
          child: Text(note, style: Theme.of(ctx).textTheme.bodySmall),
        ),
      for (final k in const [
        PreviewKind.none,
        PreviewKind.text,
        PreviewKind.markdown,
        PreviewKind.image,
        PreviewKind.pdf,
        PreviewKind.audio,
      ])
        ListTile(
          key: Key('open-as-${k.name}'),
          leading: Icon(
            k == PreviewKind.none
                ? Icons.auto_awesome_outlined
                : previewIcon(k),
          ),
          title: Text(previewLabel(k)),
          trailing: (current ?? PreviewKind.none) == k
              ? const Icon(Icons.check)
              : null,
          onTap: () => Navigator.pop(ctx, k),
        ),
    ],
  ),
);

/// A dialog with a file's type, size and dates.
Future<void> showEntryDetails(
  BuildContext context,
  EntryInfo e, {
  String? location,
  PreviewKind? rule,
}) {
  final rows = <(String, String)>[
    ('Name', e.name),
    if (location != null) ('In', location.isEmpty ? 'Top level' : location),
    if (e.kind == EntryKind.file) ...[
      ('Type', describeFileType(name: e.name)),
      ('Size', '${formatBytes(e.size.toInt())} (${e.size} bytes)'),
      ('Opens as', previewLabel(effectiveKind(e, rule: rule))),
    ],
    if (e.kind == EntryKind.folder)
      ('Contains', e.children == 1 ? '1 item' : '${e.children} items'),
    if (e.modified > 0) ('Modified', _date(e.modified)),
    if (e.executable) ('Executable', 'Yes'),
  ];
  return showDetailRows(context, title: 'Details', rows: rows);
}

Future<void> showDetailRows(
  BuildContext context, {
  required String title,
  required List<(String, String)> rows,
}) => showDialog<void>(
  context: context,
  builder: (ctx) {
    final t = Theme.of(ctx).textTheme;
    return AlertDialog(
      title: Text(title),
      content: SizedBox(
        width: 420,
        child: SingleChildScrollView(
          child: SelectionArea(
            child: Column(
              key: const Key('details-rows'),
              crossAxisAlignment: CrossAxisAlignment.start,
              mainAxisSize: MainAxisSize.min,
              children: [
                for (final (label, value) in rows)
                  Padding(
                    padding: const EdgeInsets.only(bottom: 10),
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Text(label, style: t.labelMedium),
                        Text(value, style: t.bodyMedium),
                      ],
                    ),
                  ),
              ],
            ),
          ),
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.pop(ctx),
          child: const Text('Close'),
        ),
      ],
    );
  },
);

String _date(int unix) {
  final t = DateTime.fromMillisecondsSinceEpoch(unix * 1000);
  String two(int n) => n.toString().padLeft(2, '0');
  return '${t.year}-${two(t.month)}-${two(t.day)} '
      '${two(t.hour)}:${two(t.minute)} (${relativeTime(unix)})';
}

String sortLabel(SortBy by) => switch (by) {
  SortBy.name => 'Name',
  SortBy.modified => 'Date modified',
  SortBy.type => 'Type',
  SortBy.size => 'Size',
};

/// App bar button that picks how a file list is sorted. Folders always
/// come first.
class SortButton extends StatelessWidget {
  const SortButton({super.key, required this.sort, required this.onChanged});

  final SortOrder sort;
  final ValueChanged<SortOrder> onChanged;

  @override
  Widget build(BuildContext context) {
    return PopupMenuButton<Object>(
      key: const Key('sort-menu'),
      tooltip:
          'Sort: ${sortLabel(sort.by)}, '
          '${sort.descending ? 'descending' : 'ascending'}',
      icon: const Icon(Icons.sort),
      onSelected: (v) {
        if (v is SortBy) {
          onChanged(SortOrder(by: v, descending: sort.descending));
        } else if (v is bool) {
          onChanged(SortOrder(by: sort.by, descending: v));
        }
      },
      itemBuilder: (_) => [
        for (final by in SortBy.values)
          CheckedPopupMenuItem<Object>(
            key: Key('sort-${by.name}'),
            value: by,
            checked: sort.by == by,
            child: Text(sortLabel(by)),
          ),
        const PopupMenuDivider(),
        CheckedPopupMenuItem<Object>(
          key: const Key('sort-ascending'),
          value: false,
          checked: !sort.descending,
          child: Text(_direction(sort.by, false)),
        ),
        CheckedPopupMenuItem<Object>(
          key: const Key('sort-descending'),
          value: true,
          checked: sort.descending,
          child: Text(_direction(sort.by, true)),
        ),
      ],
    );
  }

  static String _direction(SortBy by, bool descending) => switch (by) {
    SortBy.name || SortBy.type => descending ? 'Z to A' : 'A to Z',
    SortBy.modified => descending ? 'Newest first' : 'Oldest first',
    SortBy.size => descending ? 'Largest first' : 'Smallest first',
  };
}
