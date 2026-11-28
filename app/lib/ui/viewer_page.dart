import 'dart:async';
import 'dart:math' as math;
import 'dart:ui' as ui;

import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../core/errors.dart';
import '../core/format.dart';
import '../core/secure_clipboard.dart';
import '../src/rust/api/vault.dart';
import '../src/rust/api/viewer.dart';
import '../state/app_controller.dart';
import 'file_menu.dart';
import 'image_editor_page.dart';
import 'widgets/feedback.dart';

/// Turns pixels Rust decoded into a GPU image, then zeroes the Dart copy.
Future<ui.Image> pictureToImage(RenderedPicture p) async {
  final buf = await ui.ImmutableBuffer.fromUint8List(p.rgba);
  final desc = ui.ImageDescriptor.raw(
    buf,
    width: p.width,
    height: p.height,
    pixelFormat: ui.PixelFormat.rgba8888,
  );
  try {
    final codec = await desc.instantiateCodec();
    final frame = await codec.getNextFrame();
    codec.dispose();
    return frame.image;
  } finally {
    desc.dispose();
    buf.dispose();
    p.rgba.fillRange(0, p.rgba.length, 0);
  }
}

IconData previewIcon(PreviewKind k) => switch (k) {
  PreviewKind.image => Icons.image_outlined,
  PreviewKind.pdf => Icons.picture_as_pdf_outlined,
  PreviewKind.text => Icons.description_outlined,
  PreviewKind.markdown => Icons.article_outlined,
  PreviewKind.audio => Icons.audiotrack_outlined,
  PreviewKind.none => Icons.insert_drive_file_outlined,
};

/// Where the viewer's files come from: a vault or a package. Each action is
/// null where that place does not support it.
class ViewerSource {
  const ViewerSource({
    required this.open,
    required this.isClosed,
    required this.place,
    this.export,
    this.exportLabel = 'Export unencrypted…',
    this.saveText,
    this.saveImage,
    this.canSaveCopy = false,
    this.setOpenAs,
    this.openAsOf,
  });

  /// Decrypts [e] for the viewer, honouring its "Open as" choice.
  final Future<FileViewer> Function(EntryInfo e) open;

  /// True once the vault or package the files came from has closed.
  final bool Function() isClosed;

  /// "vault" or "package", for messages.
  final String place;

  /// Writes one file out unencrypted, with its confirmation.
  final Future<void> Function(EntryInfo e)? export;
  final String exportLabel;

  final Future<void> Function(EntryInfo e, String text)? saveText;

  /// Saves picture edits over the file, or as a new copy beside it.
  final Future<void> Function(
    EntryInfo e,
    FileViewer v,
    List<ImageEdit> edits,
    bool asCopy,
  )?
  saveImage;
  final bool canSaveCopy;

  /// Remembers how [e] opens (null = by its type).
  final Future<void> Function(EntryInfo e, PreviewKind? kind)? setOpenAs;

  /// The current "Open as" choice for [e], if any.
  final PreviewKind? Function(EntryInfo e)? openAsOf;
}

/// Shows files from a vault or a package, one at a time, with next and
/// previous through the folder. The plaintext stays in Rust; it is wiped
/// when this page closes or the app locks.
class ViewerPage extends StatefulWidget {
  const ViewerPage({
    super.key,
    required this.entries,
    required this.index,
    required this.source,
    this.onChanged,
  });

  /// One file from a vault.
  factory ViewerPage.vault({
    Key? key,
    required VaultSession session,
    required EntryInfo entry,
    required Future<void> Function() onExport,
  }) => ViewerPage(
    key: key,
    entries: [entry],
    index: 0,
    source: vaultSource(session, export: (_) => onExport()),
  );

  /// The files of one folder; [index] is the one shown first.
  final List<EntryInfo> entries;
  final int index;
  final ViewerSource source;

  /// Runs after something was saved, so lists can refresh.
  final VoidCallback? onChanged;

  @override
  State<ViewerPage> createState() => _ViewerPageState();
}

/// The usual [ViewerSource] for a vault.
ViewerSource vaultSource(
  VaultSession s, {
  required Future<void> Function(EntryInfo e) export,
}) => ViewerSource(
  place: 'vault',
  open: (e) => s.openViewer(id: e.id),
  isClosed: s.isLocked,
  export: export,
  saveText: (e, text) => s.saveText(id: e.id, text: text),
  saveImage: (e, v, edits, asCopy) async {
    await s.saveEditedImage(viewer: v, id: e.id, edits: edits, asCopy: asCopy);
  },
  canSaveCopy: true,
  setOpenAs: (e, k) => s.setOpenAs(id: e.id, openAs: k),
);

/// Find in a text, Markdown or PDF file: what to look for and which match
/// is current. Views report how many matches there are.
class FindController extends ChangeNotifier {
  String _query = '';
  bool _matchCase = false;
  int _current = 0;
  int _total = 0;

  String get query => _query;
  bool get matchCase => _matchCase;
  int get current => _current;
  int get total => _total;

  /// What a match is ("match", or "page" for PDFs).
  String unit = 'match';

  set query(String q) {
    if (q == _query) return;
    _query = q;
    _current = 0;
    notifyListeners();
  }

  set matchCase(bool v) {
    if (v == _matchCase) return;
    _matchCase = v;
    _current = 0;
    notifyListeners();
  }

  /// Called by the view once it has counted.
  void reportTotal(int n) {
    if (n == _total && _current < math.max(n, 1)) return;
    _total = n;
    if (_current >= n) _current = 0;
    // Views report while building; tell the find bar afterwards.
    WidgetsBinding.instance.addPostFrameCallback((_) => notifyListeners());
  }

  void next() {
    if (_total == 0) return;
    _current = (_current + 1) % _total;
    notifyListeners();
  }

  void previous() {
    if (_total == 0) return;
    _current = (_current - 1 + _total) % _total;
    notifyListeners();
  }

  /// Every place [query] appears in [text], as start offsets.
  List<int> matchesIn(String text) {
    if (_query.isEmpty) return const [];
    final hay = _matchCase ? text : text.toLowerCase();
    final needle = _matchCase ? _query : _query.toLowerCase();
    final out = <int>[];
    var i = hay.indexOf(needle);
    while (i >= 0) {
      out.add(i);
      i = hay.indexOf(needle, i + needle.length);
    }
    return out;
  }
}

/// Splits [text] into spans with every match highlighted; [currentAt] is
/// the start of the current match in this text, if it is here.
List<InlineSpan> highlightSpans(
  String text,
  List<int> starts,
  int length,
  TextStyle? style,
  ColorScheme scheme, {
  int? currentAt,
}) {
  if (starts.isEmpty) return [TextSpan(text: text, style: style)];
  final out = <InlineSpan>[];
  var at = 0;
  for (final s in starts) {
    if (s > at) out.add(TextSpan(text: text.substring(at, s), style: style));
    final current = s == currentAt;
    out.add(
      TextSpan(
        text: text.substring(s, s + length),
        style: (style ?? const TextStyle()).copyWith(
          backgroundColor: current ? scheme.tertiary : scheme.tertiaryContainer,
          color: current ? scheme.onTertiary : scheme.onTertiaryContainer,
        ),
      ),
    );
    at = s + length;
  }
  if (at < text.length) {
    out.add(TextSpan(text: text.substring(at), style: style));
  }
  return out;
}

class _ViewerPageState extends State<ViewerPage> {
  FileViewer? _viewer;
  String? _error;
  int _version = 0;
  late int _index = widget.index;
  final _find = FindController();
  bool _finding = false;
  bool _zoomed = false;
  int _turns = 0;
  final _focus = FocusNode();

  EntryInfo get _entry => widget.entries[_index];
  ViewerSource get _src => widget.source;

  @override
  void initState() {
    super.initState();
    _open();
  }

  Future<void> _open() async {
    final want = _index;
    setState(() {
      _error = null;
      _viewer?.close();
      _viewer = null;
      _zoomed = false;
      _turns = 0;
    });
    try {
      final v = await _src.open(_entry);
      if (!mounted || want != _index) {
        v.close();
        return;
      }
      setState(() {
        _viewer = v;
        _version++;
      });
    } catch (e) {
      if (mounted && want == _index) {
        setState(() => _error = describeError(e));
      }
    }
  }

  void _go(int delta) {
    final n = widget.entries.length;
    if (n < 2) return;
    setState(() => _index = (_index + delta + n) % n);
    _find.query = '';
    _open();
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    AppScope.of(context);
    if (_src.isClosed()) _viewer?.close();
  }

  @override
  void dispose() {
    _viewer?.close();
    _find.dispose();
    _focus.dispose();
    super.dispose();
  }

  Future<void> _edit(FileViewer v) async {
    final String text;
    try {
      text = await v.editableText();
    } catch (e) {
      if (mounted) showMessage(context, describeError(e), error: true);
      return;
    }
    if (!mounted) return;
    final e = _entry;
    final saved = await Navigator.of(context).push<bool>(
      MaterialPageRoute(
        builder: (_) => TextEditorPage(
          name: e.name,
          initial: text,
          onSave: (t) => _src.saveText!(e, t),
          place: _src.place,
        ),
      ),
    );
    if (saved == true && mounted) {
      showMessage(context, 'Saved into the ${_src.place}.');
      widget.onChanged?.call();
      await _open();
    }
  }

  Future<void> _editPicture(FileViewer v) async {
    final e = _entry;
    final saved = await Navigator.of(context).push<bool>(
      MaterialPageRoute(
        builder: (_) => ImageEditorPage(
          viewer: v,
          name: e.name,
          place: _src.place,
          canSaveCopy: _src.canSaveCopy,
          onSave: (edits, asCopy) => _src.saveImage!(e, v, edits, asCopy),
        ),
      ),
    );
    if (saved == true && mounted) {
      widget.onChanged?.call();
      await _open();
    }
  }

  Future<void> _openAs() async {
    final set = _src.setOpenAs;
    if (set == null) return;
    final e = _entry;
    final k = await pickOpenAs(
      context,
      name: e.name,
      current: _src.openAsOf?.call(e) ?? _viewer?.kind,
      note: _src.place == 'package'
          ? 'Remembered for every ".${fileExtension(e.name)}" file in '
                'sealed files on this device.'
          : 'Remembered for this file inside the vault.',
    );
    if (k == null || !mounted) return;
    await guarded(context, () => set(e, k == PreviewKind.none ? null : k));
    widget.onChanged?.call();
    if (mounted) await _open();
  }

  Future<void> _photoDetails(FileViewer v) async {
    final e = _entry;
    List<DetailRow> exif = const [];
    PictureSize? size;
    try {
      size = await v.imageSize();
      exif = await v.photoDetails();
    } catch (_) {}
    if (!mounted) return;
    await showDetailRows(
      context,
      title: 'Photo details',
      rows: [
        ('Name', e.name),
        if (size != null)
          (
            'Size',
            '${size.width.round()} × ${size.height.round()} pixels · '
                '${formatBytes(e.size.toInt())}',
          ),
        ('Type', describeFileType(name: e.name)),
        if (e.modified > 0) ('Modified', relativeTime(e.modified)),
        for (final r in exif) (r.label, r.value),
        if (exif.isEmpty) ('Camera details', 'None in this picture'),
      ],
    );
  }

  void _toggleFind() {
    setState(() => _finding = !_finding);
    if (!_finding) _find.query = '';
  }

  @override
  Widget build(BuildContext context) {
    final v = _viewer;
    final e = _entry;
    final kind = v?.kind;
    final textual = kind == PreviewKind.text || kind == PreviewKind.markdown;
    final canFind = textual || kind == PreviewKind.pdf;
    final canEdit = _src.saveText != null && v != null && textual;
    final picture = kind == PreviewKind.image;
    final many = widget.entries.length > 1;
    if (!canFind && _finding) _finding = false;
    Widget body = _error != null
        ? FileDetails(
            entry: e,
            message: _error,
            onExport: _src.export == null ? null : () => _src.export!(e),
            exportLabel: _src.exportLabel,
            onOpenAs: _src.setOpenAs == null ? null : _openAs,
          )
        : v == null
        ? const Center(child: CircularProgressIndicator())
        : KeyedSubtree(
            key: ValueKey(_version),
            child: switch (v.kind) {
              PreviewKind.image => ImageView(
                viewer: v,
                quarterTurns: _turns,
                onZoomed: (z) => _zoomed = z,
              ),
              PreviewKind.pdf => PdfView(
                viewer: v,
                find: _find,
                onZoomed: (z) => _zoomed = z,
              ),
              PreviewKind.markdown => MarkdownView(viewer: v, find: _find),
              PreviewKind.text => TextFileView(
                viewer: v,
                find: _find,
                onZoomed: (z) => _zoomed = z,
              ),
              PreviewKind.audio => AudioView(viewer: v, name: e.name),
              PreviewKind.none => FileDetails(
                entry: e,
                onExport: _src.export == null ? null : () => _src.export!(e),
                exportLabel: _src.exportLabel,
                onOpenAs: _src.setOpenAs == null ? null : _openAs,
              ),
            },
          );
    if (many) body = _Swipe(onSwipe: _go, blocked: () => _zoomed, child: body);
    return CallbackShortcuts(
      bindings: {
        if (many) ...{
          const SingleActivator(LogicalKeyboardKey.arrowRight): () => _go(1),
          const SingleActivator(LogicalKeyboardKey.arrowLeft): () => _go(-1),
          const SingleActivator(LogicalKeyboardKey.pageDown, alt: true): () =>
              _go(1),
          const SingleActivator(LogicalKeyboardKey.pageUp, alt: true): () =>
              _go(-1),
        },
        if (canFind) ...{
          const SingleActivator(LogicalKeyboardKey.keyF, control: true): () =>
              setState(() => _finding = true),
          const SingleActivator(LogicalKeyboardKey.keyF, meta: true): () =>
              setState(() => _finding = true),
        },
      },
      child: Focus(
        focusNode: _focus,
        autofocus: true,
        child: Scaffold(
          appBar: AppBar(
            title: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(e.name, overflow: TextOverflow.ellipsis),
                if (many)
                  Text(
                    '${_index + 1} of ${widget.entries.length}',
                    key: const Key('viewer-position'),
                    style: Theme.of(context).textTheme.bodySmall,
                  ),
              ],
            ),
            actions: [
              if (many) ...[
                IconButton(
                  key: const Key('viewer-previous'),
                  tooltip: 'Previous file',
                  icon: const Icon(Icons.chevron_left),
                  onPressed: () => _go(-1),
                ),
                IconButton(
                  key: const Key('viewer-next'),
                  tooltip: 'Next file',
                  icon: const Icon(Icons.chevron_right),
                  onPressed: () => _go(1),
                ),
              ],
              if (canFind)
                IconButton(
                  key: const Key('viewer-find'),
                  tooltip: 'Find',
                  icon: const Icon(Icons.search),
                  onPressed: _toggleFind,
                ),
              if (canEdit)
                IconButton(
                  key: const Key('viewer-edit'),
                  tooltip: 'Edit',
                  icon: const Icon(Icons.edit_outlined),
                  onPressed: () => _edit(v),
                ),
              if (picture)
                IconButton(
                  key: const Key('viewer-rotate'),
                  tooltip: 'Turn the view',
                  icon: const Icon(Icons.rotate_right),
                  onPressed: () => setState(() => _turns = (_turns + 1) % 4),
                ),
              PopupMenuButton<String>(
                key: const Key('viewer-menu'),
                onSelected: (s) => switch (s) {
                  'edit-picture' => _editPicture(v!),
                  'photo' => _photoDetails(v!),
                  'details' => showEntryDetails(context, e),
                  'open-as' => _openAs(),
                  'export' => _src.export!(e),
                  _ => null,
                },
                itemBuilder: (_) => [
                  if (picture && _src.saveImage != null)
                    const PopupMenuItem(
                      key: Key('viewer-edit-picture'),
                      value: 'edit-picture',
                      child: Text('Edit picture'),
                    ),
                  if (picture)
                    const PopupMenuItem(
                      key: Key('viewer-photo-details'),
                      value: 'photo',
                      child: Text('Photo details'),
                    )
                  else
                    const PopupMenuItem(
                      value: 'details',
                      child: Text('Details'),
                    ),
                  if (_src.setOpenAs != null)
                    const PopupMenuItem(
                      key: Key('viewer-open-as'),
                      value: 'open-as',
                      child: Text('Open as…'),
                    ),
                  if (_src.export != null)
                    PopupMenuItem(
                      key: const Key('viewer-export'),
                      value: 'export',
                      child: Text(_src.exportLabel),
                    ),
                ],
              ),
            ],
            bottom: _finding
                ? PreferredSize(
                    preferredSize: const Size.fromHeight(56),
                    child: _FindBar(find: _find, onClose: _toggleFind),
                  )
                : null,
          ),
          body: body,
        ),
      ),
    );
  }
}

/// A quick sideways flick moves to the next or previous file. It watches raw
/// pointer events so it never takes a drag away from scrolling or zooming.
class _Swipe extends StatefulWidget {
  const _Swipe({
    required this.onSwipe,
    required this.blocked,
    required this.child,
  });

  final ValueChanged<int> onSwipe;
  final bool Function() blocked;
  final Widget child;

  @override
  State<_Swipe> createState() => _SwipeState();
}

class _SwipeState extends State<_Swipe> {
  final Map<int, Offset> _down = {};
  Offset? _start;
  DateTime? _at;
  bool _multi = false;

  @override
  Widget build(BuildContext context) {
    return Listener(
      onPointerDown: (e) {
        _down[e.pointer] = e.position;
        if (_down.length == 1) {
          _start = e.position;
          _at = DateTime.now();
          _multi = false;
        } else {
          _multi = true;
        }
      },
      onPointerUp: (e) {
        _down.remove(e.pointer);
        final start = _start;
        final at = _at;
        if (_down.isNotEmpty || start == null || at == null || _multi) return;
        _start = null;
        final d = e.position - start;
        final quick = DateTime.now().difference(at).inMilliseconds < 450;
        if (!quick || widget.blocked()) return;
        if (d.dx.abs() > 90 && d.dy.abs() < d.dx.abs() * 0.5) {
          widget.onSwipe(d.dx < 0 ? 1 : -1);
        }
      },
      onPointerCancel: (e) {
        _down.remove(e.pointer);
        _start = null;
      },
      child: widget.child,
    );
  }
}

class _FindBar extends StatefulWidget {
  const _FindBar({required this.find, required this.onClose});
  final FindController find;
  final VoidCallback onClose;

  @override
  State<_FindBar> createState() => _FindBarState();
}

class _FindBarState extends State<_FindBar> {
  late final _text = TextEditingController(text: widget.find.query);
  Timer? _debounce;

  @override
  void dispose() {
    _debounce?.cancel();
    _text.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final f = widget.find;
    return Padding(
      padding: const EdgeInsets.fromLTRB(12, 0, 4, 8),
      child: ListenableBuilder(
        listenable: f,
        builder: (context, _) {
          final count = f.query.isEmpty
              ? ''
              : f.total == 0
              ? 'Not found'
              : f.unit == 'page'
              ? 'Page ${f.current + 1} of ${f.total}'
              : '${f.current + 1} of ${f.total}';
          return Row(
            children: [
              Expanded(
                child: CallbackShortcuts(
                  bindings: {
                    const SingleActivator(LogicalKeyboardKey.enter): f.next,
                    const SingleActivator(
                      LogicalKeyboardKey.enter,
                      shift: true,
                    ): f.previous,
                    const SingleActivator(LogicalKeyboardKey.escape):
                        widget.onClose,
                  },
                  child: TextField(
                    key: const Key('find-field'),
                    controller: _text,
                    autofocus: true,
                    autocorrect: false,
                    enableSuggestions: false,
                    enableIMEPersonalizedLearning: false,
                    decoration: InputDecoration(
                      isDense: true,
                      hintText: 'Find',
                      prefixIcon: const Icon(Icons.search, size: 20),
                      suffixText: count,
                      border: const OutlineInputBorder(),
                    ),
                    onChanged: (q) {
                      _debounce?.cancel();
                      _debounce = Timer(
                        const Duration(milliseconds: 250),
                        () => f.query = q,
                      );
                    },
                    onSubmitted: (_) => f.next(),
                  ),
                ),
              ),
              IconButton(
                key: const Key('find-case'),
                tooltip: f.matchCase ? 'Match case: on' : 'Match case: off',
                isSelected: f.matchCase,
                icon: const Icon(Icons.text_fields),
                onPressed: () => f.matchCase = !f.matchCase,
              ),
              IconButton(
                key: const Key('find-previous'),
                tooltip: 'Previous',
                icon: const Icon(Icons.keyboard_arrow_up),
                onPressed: f.total == 0 ? null : f.previous,
              ),
              IconButton(
                key: const Key('find-next'),
                tooltip: 'Next',
                icon: const Icon(Icons.keyboard_arrow_down),
                onPressed: f.total == 0 ? null : f.next,
              ),
              IconButton(
                tooltip: 'Close',
                icon: const Icon(Icons.close),
                onPressed: widget.onClose,
              ),
            ],
          );
        },
      ),
    );
  }
}

/// For files the viewer can't show: type, size, date and an export button.
class FileDetails extends StatelessWidget {
  const FileDetails({
    super.key,
    required this.entry,
    required this.onExport,
    this.exportLabel = 'Export unencrypted…',
    this.onOpenAs,
    this.message,
  });

  final EntryInfo entry;
  final Future<void> Function()? onExport;
  final String exportLabel;
  final Future<void> Function()? onOpenAs;
  final String? message;

  @override
  Widget build(BuildContext context) {
    final t = Theme.of(context).textTheme;
    final kind = previewForName(name: entry.name);
    return ListView(
      padding: const EdgeInsets.all(24),
      children: [
        Icon(
          previewIcon(kind),
          size: 72,
          color: Theme.of(context).colorScheme.primary,
        ),
        const SizedBox(height: 12),
        Text(entry.name, textAlign: TextAlign.center, style: t.titleMedium),
        const SizedBox(height: 4),
        Text(
          [
            describeFileType(name: entry.name),
            formatBytes(entry.size.toInt()),
            if (entry.modified > 0) relativeTime(entry.modified),
          ].join(' · '),
          textAlign: TextAlign.center,
          style: t.bodySmall,
        ),
        const SizedBox(height: 24),
        Text(
          message ??
              (onExport != null
                  ? 'Atropos keeps this file encrypted but cannot tell what '
                        'it is. Open it as text or another type, or export it '
                        'to use it in another app.'
                  : 'Atropos cannot tell what kind of file this is.'),
          key: const Key('viewer-message'),
          textAlign: TextAlign.center,
        ),
        const SizedBox(height: 16),
        Wrap(
          alignment: WrapAlignment.center,
          spacing: 12,
          runSpacing: 8,
          children: [
            if (onOpenAs != null)
              FilledButton.tonalIcon(
                key: const Key('details-open-as'),
                onPressed: onOpenAs,
                icon: const Icon(Icons.file_open_outlined),
                label: const Text('Open as…'),
              ),
            if (onExport != null)
              OutlinedButton.icon(
                onPressed: onExport,
                icon: const Icon(Icons.file_download_outlined),
                label: Text(exportLabel),
              ),
          ],
        ),
      ],
    );
  }
}

class _Failed extends StatelessWidget {
  const _Failed(this.error);
  final Object error;

  @override
  Widget build(BuildContext context) => Center(
    child: Padding(
      padding: const EdgeInsets.all(24),
      child: Text(describeError(error), textAlign: TextAlign.center),
    ),
  );
}

// ---- pictures ------------------------------------------------------------

/// A picture with pinch zoom. It is rendered at twice the screen size so
/// zooming stays sharp, and again at full size once zoomed further.
class ImageView extends StatefulWidget {
  const ImageView({
    super.key,
    required this.viewer,
    this.quarterTurns = 0,
    this.onZoomed,
  });
  final FileViewer viewer;

  /// Turns the view (not the file) by 90 degree steps.
  final int quarterTurns;
  final ValueChanged<bool>? onZoomed;

  @override
  State<ImageView> createState() => _ImageViewState();
}

class _ImageViewState extends State<ImageView> {
  final _zoom = TransformationController();
  ui.Image? _image;
  Object? _error;
  Size? _full;
  Size? _box;
  int _wanted = 0;
  bool _sharp = false;

  /// Frame times of an animation (null for a still picture).
  List<int>? _delays;
  final Map<int, ui.Image> _frames = {};
  int _frame = 0;
  bool _playing = true;
  Timer? _tick;

  @override
  void initState() {
    super.initState();
    _zoom.addListener(_onZoom);
    _start();
  }

  Future<void> _start() async {
    try {
      final s = await widget.viewer.imageSize();
      final d = await widget.viewer.frameDelays();
      if (!mounted) return;
      setState(() {
        _full = Size(s.width, s.height);
        if (d.length > 1) _delays = d.toList();
      });
    } catch (e) {
      if (mounted) setState(() => _error = e);
    }
  }

  bool _wasZoomed = false;

  void _onZoom() {
    final zoomed = _zoom.value.getMaxScaleOnAxis() > 1.01;
    if (zoomed != _wasZoomed) {
      _wasZoomed = zoomed;
      widget.onZoomed?.call(zoomed);
    }
    if (_delays != null) return;
    if (!_sharp && _zoom.value.getMaxScaleOnAxis() > 1.8 && _box != null) {
      _sharp = true;
      _render(_box!, factor: 6);
    }
  }

  @override
  void didUpdateWidget(ImageView old) {
    super.didUpdateWidget(old);
    if (old.quarterTurns != widget.quarterTurns) {
      _zoom.value = Matrix4.identity();
      final box = _box;
      if (box != null && widget.quarterTurns.isOdd != old.quarterTurns.isOdd) {
        WidgetsBinding.instance.addPostFrameCallback((_) {
          if (mounted) _render(box, factor: _sharp ? 6 : 2);
        });
      }
    }
  }

  Future<void> _render(Size box, {double factor = 2}) async {
    if (widget.quarterTurns.isOdd) box = box.flipped;
    if (_delays != null) return _restartAnimation(box);
    final dpr = MediaQuery.devicePixelRatioOf(context);
    final ticket = ++_wanted;
    try {
      final p = await widget.viewer.renderImage(
        maxWidth: (box.width * dpr * factor).ceil(),
        maxHeight: (box.height * dpr * factor).ceil(),
      );
      final img = await pictureToImage(p);
      if (!mounted || ticket != _wanted) {
        img.dispose();
        return;
      }
      setState(() {
        _image?.dispose();
        _image = img;
      });
    } catch (e) {
      if (mounted) setState(() => _error = e);
    }
  }

  // Animations: every frame is rendered once at screen size and kept, then
  // shown in turn. Rust already capped the total size of all frames.

  void _clearFrames() {
    _tick?.cancel();
    for (final f in _frames.values) {
      f.dispose();
    }
    _frames.clear();
    _image = null;
  }

  Future<void> _restartAnimation(Size box) async {
    final ticket = ++_wanted;
    setState(_clearFrames);
    _frame = 0;
    await _show(0, box, ticket);
  }

  Future<ui.Image?> _frameImage(int i, Size box, int ticket) async {
    final have = _frames[i];
    if (have != null) return have;
    final dpr = MediaQuery.devicePixelRatioOf(context);
    final p = await widget.viewer.renderFrame(
      index: i,
      maxWidth: (box.width * dpr).ceil(),
      maxHeight: (box.height * dpr).ceil(),
    );
    final img = await pictureToImage(p);
    if (!mounted || ticket != _wanted) {
      img.dispose();
      return null;
    }
    _frames[i] = img;
    return img;
  }

  Future<void> _show(int i, Size box, int ticket) async {
    final delays = _delays!;
    try {
      final img = await _frameImage(i, box, ticket);
      if (img == null || !mounted || ticket != _wanted) return;
      setState(() {
        _frame = i;
        _image = img;
      });
      _tick?.cancel();
      if (_playing) {
        _tick = Timer(Duration(milliseconds: delays[i]), () {
          if (mounted && ticket == _wanted) {
            _show((i + 1) % delays.length, box, ticket);
          }
        });
      }
    } catch (e) {
      if (mounted) setState(() => _error = e);
    }
  }

  void _togglePlay() {
    setState(() => _playing = !_playing);
    final box = _box;
    if (_playing && box != null) {
      _show((_frame + 1) % _delays!.length, box, _wanted);
    } else {
      _tick?.cancel();
    }
  }

  @override
  void dispose() {
    _zoom.dispose();
    _tick?.cancel();
    if (_delays != null) {
      _clearFrames();
    } else {
      _image?.dispose();
    }
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    if (_error != null) return _Failed(_error!);
    return LayoutBuilder(
      builder: (context, c) {
        final box = c.biggest;
        if (_box != box && _full != null) {
          _box = box;
          WidgetsBinding.instance.addPostFrameCallback((_) {
            if (mounted) _render(box, factor: _sharp ? 6 : 2);
          });
        }
        final img = _image;
        if (img == null) {
          return const Center(child: CircularProgressIndicator());
        }
        return Stack(
          children: [
            InteractiveViewer(
              transformationController: _zoom,
              maxScale: 12,
              child: SizedBox.expand(
                child: RotatedBox(
                  quarterTurns: widget.quarterTurns,
                  child: RawImage(
                    key: const Key('viewer-image'),
                    image: img,
                    fit: BoxFit.contain,
                    filterQuality: FilterQuality.medium,
                  ),
                ),
              ),
            ),
            if (_delays != null)
              Positioned(
                right: 12,
                bottom: 12,
                child: FloatingActionButton.small(
                  key: const Key('viewer-animation'),
                  tooltip: _playing ? 'Pause' : 'Play',
                  onPressed: _togglePlay,
                  child: Icon(_playing ? Icons.pause : Icons.play_arrow),
                ),
              ),
          ],
        );
      },
    );
  }
}

// ---- PDF -----------------------------------------------------------------

/// Pages one under another, zoomed in place: pinch, double tap, Ctrl +
/// scroll wheel or the + and - buttons. Only pages near the screen keep a
/// rendering, sharp for the current zoom.
class PdfView extends StatefulWidget {
  const PdfView({super.key, required this.viewer, this.find, this.onZoomed});
  final FileViewer viewer;
  final FindController? find;
  final ValueChanged<bool>? onZoomed;

  @override
  State<PdfView> createState() => _PdfViewState();
}

class _PdfViewState extends State<PdfView> {
  static const _gap = 12.0;
  static const _maxZoom = 5.0;

  List<PictureSize>? _pages;
  Object? _error;
  final Map<int, ui.Image> _images = {};
  final Map<int, int> _imageWidth = {};
  final Set<int> _pending = {};
  final _scroll = ScrollController();
  final _hScroll = ScrollController();
  double _zoom = 1;
  Timer? _sharpen;
  int _page = 0;
  double _cssWidth = 0;

  // Pinch tracking.
  final Map<int, Offset> _pointers = {};
  double? _pinchStart;
  double _zoomAtPinch = 1;

  // Find.
  List<PdfMatch> _hits = const [];
  String _searched = '';
  bool _searchedCase = false;
  int _shownHit = -1;

  @override
  void initState() {
    super.initState();
    widget.viewer.pdfPages().then(
      (p) {
        if (mounted) setState(() => _pages = p);
      },
      onError: (Object e) {
        if (mounted) setState(() => _error = e);
      },
    );
    _scroll.addListener(_trackPage);
    widget.find?.unit = 'page';
    widget.find?.addListener(_onFind);
  }

  @override
  void dispose() {
    widget.find?.removeListener(_onFind);
    widget.find?.unit = 'match';
    _sharpen?.cancel();
    _scroll.dispose();
    _hScroll.dispose();
    for (final i in _images.values) {
      i.dispose();
    }
    super.dispose();
  }

  double _pageHeight(PictureSize s) =>
      _cssWidth * _zoom * s.height / s.width + _gap;

  double _offsetOf(int index) {
    final pages = _pages!;
    var y = _gap / 2;
    for (var i = 0; i < index && i < pages.length; i++) {
      y += _pageHeight(pages[i]);
    }
    return y;
  }

  void _trackPage() {
    final pages = _pages;
    if (pages == null || pages.isEmpty) return;
    final mid = _scroll.offset + _scroll.position.viewportDimension / 3;
    var y = 0.0;
    var p = pages.length - 1;
    for (var i = 0; i < pages.length; i++) {
      y += _pageHeight(pages[i]);
      if (y > mid) {
        p = i;
        break;
      }
    }
    if (p != _page) setState(() => _page = p);
  }

  void _jumpTo(int index) {
    if (!_scroll.hasClients) return;
    final max = _scroll.position.maxScrollExtent;
    _scroll.jumpTo(_offsetOf(index).clamp(0, max));
  }

  Future<void> _onFind() async {
    final f = widget.find!;
    if (f.query != _searched || f.matchCase != _searchedCase) {
      _searched = f.query;
      _searchedCase = f.matchCase;
      _shownHit = -1;
      if (f.query.trim().isEmpty) {
        setState(() => _hits = const []);
        f.reportTotal(0);
        return;
      }
      final q = f.query;
      try {
        final hits = await widget.viewer.pdfSearch(
          query: q,
          matchCase: f.matchCase,
        );
        if (!mounted || q != f.query) return;
        setState(() => _hits = hits);
        f.reportTotal(hits.length);
      } catch (_) {
        f.reportTotal(0);
        return;
      }
    }
    if (_hits.isNotEmpty && f.current != _shownHit) {
      _shownHit = f.current;
      _jumpTo(_hits[f.current].page);
      setState(() {});
    }
  }

  void _setZoom(double z, {Offset? focus}) {
    z = z.clamp(1.0, _maxZoom);
    if ((z - _zoom).abs() < 0.001) return;
    // Keep the point under the fingers (or the screen centre) in place.
    final vy = _scroll.hasClients ? _scroll.offset : 0.0;
    final vx = _hScroll.hasClients ? _hScroll.offset : 0.0;
    final fy = focus?.dy ?? 0;
    final fx = focus?.dx ?? 0;
    final ratio = z / _zoom;
    setState(() => _zoom = z);
    widget.onZoomed?.call(z > 1.01);
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted) return;
      if (_scroll.hasClients) {
        _scroll.jumpTo(
          ((vy + fy) * ratio - fy).clamp(0, _scroll.position.maxScrollExtent),
        );
      }
      if (_hScroll.hasClients) {
        _hScroll.jumpTo(
          ((vx + fx) * ratio - fx).clamp(0, _hScroll.position.maxScrollExtent),
        );
      }
    });
    _sharpen?.cancel();
    _sharpen = Timer(const Duration(milliseconds: 300), () {
      if (mounted) setState(() {});
    });
  }

  Future<void> _load(int index, int width) async {
    if (_pending.contains(index)) return;
    _pending.add(index);
    try {
      final p = await widget.viewer.renderPage(
        index: index,
        maxWidth: width,
        maxHeight: 8192,
      );
      final img = await pictureToImage(p);
      if (!mounted) {
        img.dispose();
        return;
      }
      setState(() {
        _images.remove(index)?.dispose();
        _images[index] = img;
        _imageWidth[index] = width;
        // Keep memory in check: forget pages far from this one.
        final keep = _zoom > 1.5 ? 2 : 4;
        final far = _images.keys
            .where((k) => (k - index).abs() > keep)
            .toList();
        for (final k in far) {
          _images.remove(k)?.dispose();
          _imageWidth.remove(k);
        }
      });
    } catch (e) {
      if (mounted) setState(() => _error = e);
    } finally {
      _pending.remove(index);
    }
  }

  @override
  Widget build(BuildContext context) {
    if (_error != null) return _Failed(_error!);
    final pages = _pages;
    if (pages == null) return const Center(child: CircularProgressIndicator());
    final scheme = Theme.of(context).colorScheme;
    return LayoutBuilder(
      builder: (context, c) {
        final dpr = MediaQuery.devicePixelRatioOf(context);
        _cssWidth = math.min(c.maxWidth - 24, 1000.0);
        final pageWidth = _cssWidth * _zoom;
        // Rendered pixels follow the zoom once it settles, up to a cap.
        final settled = _sharpen?.isActive != true;
        final width = math.min(pageWidth * dpr, 4096.0).round();
        final contentWidth = math.max(c.maxWidth, pageWidth + 24);
        final pinching = _pointers.length > 1;
        final hitPages = {for (final h in _hits) h.page};
        final current = _hits.isEmpty || widget.find == null
            ? -1
            : _hits[widget.find!.current.clamp(0, _hits.length - 1)].page;
        Widget list = ListView.builder(
          key: const Key('viewer-pdf'),
          controller: _scroll,
          physics: pinching ? const NeverScrollableScrollPhysics() : null,
          padding: const EdgeInsets.symmetric(vertical: _gap / 2),
          itemCount: pages.length,
          itemBuilder: (context, i) {
            final size = pages[i];
            final img = _images[i];
            if (settled && (img == null || _imageWidth[i] != width)) {
              _load(i, width);
            }
            final mark = i == current
                ? scheme.tertiary
                : hitPages.contains(i)
                ? scheme.tertiaryContainer
                : null;
            return Center(
              child: Padding(
                padding: const EdgeInsets.symmetric(vertical: _gap / 2),
                child: Container(
                  key: Key('pdf-page-$i'),
                  width: pageWidth,
                  height: pageWidth * size.height / size.width,
                  decoration: BoxDecoration(
                    color: Colors.white,
                    border: mark == null
                        ? null
                        : Border.all(color: mark, width: 3),
                    boxShadow: [
                      BoxShadow(
                        blurRadius: 3,
                        color: Colors.black.withValues(alpha: 0.2),
                      ),
                    ],
                  ),
                  child: img == null
                      ? null
                      : RawImage(image: img, fit: BoxFit.fill),
                ),
              ),
            );
          },
        );
        if (contentWidth > c.maxWidth) {
          list = Scrollbar(
            controller: _hScroll,
            child: SingleChildScrollView(
              controller: _hScroll,
              scrollDirection: Axis.horizontal,
              physics: pinching ? const NeverScrollableScrollPhysics() : null,
              child: SizedBox(
                width: contentWidth,
                height: c.maxHeight,
                child: list,
              ),
            ),
          );
        }
        final hit = current >= 0 ? _hits[widget.find!.current] : null;
        return ColoredBox(
          color: scheme.surfaceContainerHighest,
          child: Stack(
            children: [
              Listener(
                onPointerDown: (e) {
                  _pointers[e.pointer] = e.localPosition;
                  if (_pointers.length == 2) {
                    final p = _pointers.values.toList();
                    _pinchStart = (p[0] - p[1]).distance;
                    _zoomAtPinch = _zoom;
                    setState(() {});
                  }
                },
                onPointerMove: (e) {
                  if (!_pointers.containsKey(e.pointer)) return;
                  _pointers[e.pointer] = e.localPosition;
                  final start = _pinchStart;
                  if (_pointers.length == 2 && start != null && start > 0) {
                    final p = _pointers.values.toList();
                    final d = (p[0] - p[1]).distance;
                    _setZoom(
                      _zoomAtPinch * d / start,
                      focus: (p[0] + p[1]) / 2,
                    );
                  }
                },
                onPointerUp: (e) {
                  _pointers.remove(e.pointer);
                  if (_pointers.length < 2) {
                    _pinchStart = null;
                    setState(() {});
                  }
                },
                onPointerCancel: (e) {
                  _pointers.remove(e.pointer);
                  if (_pointers.length < 2) _pinchStart = null;
                },
                onPointerSignal: (e) {
                  if (e is PointerScrollEvent &&
                      (HardwareKeyboard.instance.isControlPressed ||
                          HardwareKeyboard.instance.isMetaPressed)) {
                    GestureBinding.instance.pointerSignalResolver.register(e, (
                      _,
                    ) {
                      _setZoom(
                        _zoom * (e.scrollDelta.dy < 0 ? 1.15 : 1 / 1.15),
                        focus: e.localPosition,
                      );
                    });
                  }
                },
                child: GestureDetector(
                  onDoubleTapDown: (d) => _doubleTapAt = d.localPosition,
                  onDoubleTap: () =>
                      _setZoom(_zoom > 1.2 ? 1 : 2.5, focus: _doubleTapAt),
                  child: list,
                ),
              ),
              Positioned(
                left: 12,
                bottom: 12,
                child: Material(
                  color: scheme.inverseSurface.withValues(alpha: 0.85),
                  borderRadius: BorderRadius.circular(16),
                  child: Padding(
                    padding: const EdgeInsets.symmetric(
                      horizontal: 12,
                      vertical: 6,
                    ),
                    child: Text(
                      'Page ${_page + 1} of ${pages.length}'
                      '${_zoom > 1.01 ? ' · ${(_zoom * 100).round()}%' : ''}',
                      key: const Key('pdf-page-label'),
                      style: TextStyle(color: scheme.onInverseSurface),
                    ),
                  ),
                ),
              ),
              if (hit != null)
                Positioned(
                  left: 12,
                  right: 72,
                  top: 8,
                  child: Material(
                    elevation: 2,
                    color: scheme.tertiaryContainer,
                    borderRadius: BorderRadius.circular(8),
                    child: Padding(
                      padding: const EdgeInsets.all(8),
                      child: Text(
                        'Page ${hit.page + 1}'
                        '${hit.count > 1 ? ' (${hit.count} times)' : ''}: '
                        '${hit.snippet}',
                        key: const Key('pdf-find-snippet'),
                        maxLines: 2,
                        overflow: TextOverflow.ellipsis,
                        style: TextStyle(color: scheme.onTertiaryContainer),
                      ),
                    ),
                  ),
                ),
              Positioned(
                right: 12,
                bottom: 12,
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    FloatingActionButton.small(
                      key: const Key('pdf-zoom-in'),
                      heroTag: null,
                      tooltip: 'Zoom in',
                      onPressed: _zoom >= _maxZoom
                          ? null
                          : () => _setZoom(
                              _zoom * 1.5,
                              focus: Offset(c.maxWidth / 2, c.maxHeight / 2),
                            ),
                      child: const Icon(Icons.zoom_in),
                    ),
                    const SizedBox(height: 8),
                    FloatingActionButton.small(
                      key: const Key('pdf-zoom-out'),
                      heroTag: null,
                      tooltip: 'Zoom out',
                      onPressed: _zoom <= 1
                          ? null
                          : () => _setZoom(
                              _zoom / 1.5,
                              focus: Offset(c.maxWidth / 2, c.maxHeight / 2),
                            ),
                      child: const Icon(Icons.zoom_out),
                    ),
                  ],
                ),
              ),
            ],
          ),
        );
      },
    );
  }

  Offset? _doubleTapAt;
}

// ---- text ----------------------------------------------------------------

class TextFileView extends StatefulWidget {
  const TextFileView({
    super.key,
    required this.viewer,
    this.find,
    this.onZoomed,
  });
  final FileViewer viewer;
  final FindController? find;

  /// Reports when side-scrolling lines are on (swipes then scroll instead).
  final ValueChanged<bool>? onZoomed;

  @override
  State<TextFileView> createState() => _TextFileViewState();
}

class _TextFileViewState extends State<TextFileView> {
  static const _lineHeight = 13 * 1.4;

  TextView? _text;
  Object? _error;
  bool _wrap = true;
  final _scroll = ScrollController();
  final _currentKey = GlobalKey();

  /// (line, start) of every match.
  List<(int, int)> _matches = const [];
  String _for = '';
  bool _forCase = false;
  int _shown = -1;

  @override
  void initState() {
    super.initState();
    widget.viewer.text().then(
      (t) {
        if (!mounted) return;
        setState(() => _text = t);
        _onFind();
      },
      onError: (Object e) {
        if (mounted) setState(() => _error = e);
      },
    );
    widget.find?.addListener(_onFind);
  }

  @override
  void dispose() {
    widget.find?.removeListener(_onFind);
    _scroll.dispose();
    super.dispose();
  }

  void _onFind() {
    final f = widget.find;
    final t = _text;
    if (f == null || t == null) return;
    if (f.query != _for || f.matchCase != _forCase) {
      _for = f.query;
      _forCase = f.matchCase;
      _shown = -1;
      final found = <(int, int)>[];
      for (var i = 0; i < t.lines.length && found.length < 10000; i++) {
        for (final s in f.matchesIn(t.lines[i])) {
          found.add((i, s));
        }
      }
      setState(() => _matches = found);
      f.reportTotal(found.length);
    }
    if (_matches.isNotEmpty && f.current != _shown) {
      _shown = f.current;
      setState(() {});
      _reveal(_matches[f.current.clamp(0, _matches.length - 1)].$1);
    }
  }

  /// Scrolls line [i] into view: a guess first, then exactly once built.
  void _reveal(int i) {
    if (!_scroll.hasClients) return;
    final pos = _scroll.position;
    final guess = (i * _lineHeight - pos.viewportDimension / 3).clamp(
      0.0,
      pos.maxScrollExtent,
    );
    final ctx = _currentKey.currentContext;
    if (ctx != null) {
      Scrollable.ensureVisible(ctx, alignment: 0.3);
      return;
    }
    _scroll.jumpTo(guess);
    WidgetsBinding.instance.addPostFrameCallback((_) {
      final c = _currentKey.currentContext;
      if (c != null && c.mounted) Scrollable.ensureVisible(c, alignment: 0.3);
    });
  }

  @override
  Widget build(BuildContext context) {
    if (_error != null) return _Failed(_error!);
    final t = _text;
    if (t == null) return const Center(child: CircularProgressIndicator());
    final scheme = Theme.of(context).colorScheme;
    final style = TextStyle(
      fontFamily: 'monospace',
      fontFamilyFallback: const ['Menlo', 'Consolas', 'Courier New'],
      fontSize: 13,
      height: 1.4,
      color: scheme.onSurface,
    );
    final f = widget.find;
    final byLine = <int, List<int>>{};
    for (final (line, start) in _matches) {
      (byLine[line] ??= []).add(start);
    }
    final cur = _matches.isEmpty || f == null
        ? null
        : _matches[f.current.clamp(0, _matches.length - 1)];
    final digits = '${t.lines.length}'.length;
    Widget list = ListView.builder(
      key: const Key('viewer-text'),
      controller: _scroll,
      padding: const EdgeInsets.fromLTRB(8, 8, 16, 24),
      itemExtent: _wrap ? null : _lineHeight,
      itemCount: t.lines.length + (t.truncated ? 1 : 0),
      itemBuilder: (context, i) {
        if (i == t.lines.length) {
          return Padding(
            padding: const EdgeInsets.all(16),
            child: Text(
              'The rest of this file is not shown because it is very long. '
              'Export it to read all of it.',
              style: TextStyle(color: scheme.error),
            ),
          );
        }
        final starts = byLine[i];
        final line = starts == null
            ? TextSpan(text: t.lines[i], style: style)
            : TextSpan(
                children: highlightSpans(
                  t.lines[i],
                  starts,
                  f!.query.length,
                  style,
                  scheme,
                  currentAt: cur?.$1 == i ? cur!.$2 : null,
                ),
              );
        return Row(
          key: cur?.$1 == i ? _currentKey : null,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            SizedBox(
              width: digits * 8.5 + 12,
              child: Text(
                '${i + 1}',
                textAlign: TextAlign.right,
                style: style.copyWith(color: scheme.outline),
              ),
            ),
            const SizedBox(width: 12),
            if (_wrap)
              Expanded(child: Text.rich(line))
            else
              Text.rich(line, softWrap: false),
          ],
        );
      },
    );
    if (!_wrap) {
      list = SingleChildScrollView(
        scrollDirection: Axis.horizontal,
        child: SizedBox(width: 4000, child: list),
      );
    }
    // Long-press or drag to select; Copy goes through the clipboard guard.
    list = SelectionArea(
      contextMenuBuilder: (context, sel) =>
          AdaptiveTextSelectionToolbar.buttonItems(
            anchors: sel.contextMenuAnchors,
            buttonItems: SecureClipboard.guard(
              context,
              sel.contextMenuButtonItems,
            ),
          ),
      child: list,
    );
    return Stack(
      children: [
        list,
        Positioned(
          right: 12,
          bottom: 12,
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              FloatingActionButton.small(
                key: const Key('viewer-copy-all'),
                heroTag: null,
                tooltip: 'Copy all',
                onPressed: () async {
                  await SecureClipboard.copy(t.lines.join('\n'));
                  if (context.mounted) {
                    showMessage(
                      context,
                      t.truncated
                          ? 'Copied the part that is shown. Atropos clears '
                                'the clipboard after 60 seconds or when it locks.'
                          : SecureClipboard.notice,
                    );
                  }
                },
                child: const Icon(Icons.copy_all_outlined),
              ),
              const SizedBox(height: 8),
              FloatingActionButton.small(
                heroTag: null,
                tooltip: _wrap ? 'Don\'t wrap lines' : 'Wrap lines',
                onPressed: () {
                  setState(() => _wrap = !_wrap);
                  widget.onZoomed?.call(!_wrap);
                },
                child: Icon(_wrap ? Icons.wrap_text : Icons.notes),
              ),
            ],
          ),
        ),
      ],
    );
  }
}

class MarkdownView extends StatefulWidget {
  const MarkdownView({super.key, required this.viewer, this.find});
  final FileViewer viewer;
  final FindController? find;

  @override
  State<MarkdownView> createState() => _MarkdownViewState();
}

class _MarkdownViewState extends State<MarkdownView> {
  List<MdBlock>? _blocks;
  Object? _error;
  final _scroll = ScrollController();
  final _currentKey = GlobalKey();

  /// (block, span, start) of every match.
  List<(int, int, int)> _matches = const [];
  String _for = '';
  bool _forCase = false;
  int _shown = -1;

  @override
  void initState() {
    super.initState();
    widget.viewer.markdown().then(
      (b) {
        if (!mounted) return;
        setState(() => _blocks = b);
        _onFind();
      },
      onError: (Object e) {
        if (mounted) setState(() => _error = e);
      },
    );
    widget.find?.addListener(_onFind);
  }

  @override
  void dispose() {
    widget.find?.removeListener(_onFind);
    _scroll.dispose();
    super.dispose();
  }

  void _onFind() {
    final f = widget.find;
    final blocks = _blocks;
    if (f == null || blocks == null) return;
    if (f.query != _for || f.matchCase != _forCase) {
      _for = f.query;
      _forCase = f.matchCase;
      _shown = -1;
      final found = <(int, int, int)>[];
      for (var b = 0; b < blocks.length; b++) {
        final spans = blocks[b].spans;
        for (var s = 0; s < spans.length; s++) {
          for (final at in f.matchesIn(mdSpanText(spans[s]))) {
            found.add((b, s, at));
          }
        }
      }
      setState(() => _matches = found);
      f.reportTotal(found.length);
    }
    if (_matches.isNotEmpty && f.current != _shown) {
      _shown = f.current;
      setState(() {});
      final b = _matches[f.current.clamp(0, _matches.length - 1)].$1;
      final ctx = _currentKey.currentContext;
      if (ctx != null) {
        Scrollable.ensureVisible(ctx, alignment: 0.3);
      } else if (_scroll.hasClients) {
        final pos = _scroll.position;
        _scroll.jumpTo(
          (pos.maxScrollExtent * b / math.max(blocks.length - 1, 1)).clamp(
            0,
            pos.maxScrollExtent,
          ),
        );
        WidgetsBinding.instance.addPostFrameCallback((_) {
          final c = _currentKey.currentContext;
          if (c != null && c.mounted) {
            Scrollable.ensureVisible(c, alignment: 0.3);
          }
        });
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    if (_error != null) return _Failed(_error!);
    final blocks = _blocks;
    if (blocks == null) return const Center(child: CircularProgressIndicator());
    final f = widget.find;
    final cur = _matches.isEmpty || f == null
        ? null
        : _matches[f.current.clamp(0, _matches.length - 1)];
    final byBlock = <int, Map<int, List<int>>>{};
    for (final (b, s, at) in _matches) {
      ((byBlock[b] ??= {})[s] ??= []).add(at);
    }
    return Align(
      alignment: Alignment.topCenter,
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: 760),
        child: SelectionArea(
          contextMenuBuilder: (context, sel) =>
              AdaptiveTextSelectionToolbar.buttonItems(
                anchors: sel.contextMenuAnchors,
                buttonItems: SecureClipboard.guard(
                  context,
                  sel.contextMenuButtonItems,
                ),
              ),
          child: ListView.builder(
            key: const Key('viewer-markdown'),
            controller: _scroll,
            padding: const EdgeInsets.fromLTRB(20, 8, 20, 32),
            itemCount: blocks.length,
            itemBuilder: (context, i) => KeyedSubtree(
              key: cur?.$1 == i ? _currentKey : null,
              child: MarkdownBlock(
                block: blocks[i],
                marks: byBlock[i],
                markLength: f?.query.length ?? 0,
                current: cur?.$1 == i ? (cur!.$2, cur.$3) : null,
              ),
            ),
          ),
        ),
      ),
    );
  }
}

/// The text a Markdown span shows (links show their address too).
String mdSpanText(MdSpan s) =>
    s.link != null && s.link != s.text ? '${s.text} (${s.link})' : s.text;

/// Release notes and other Markdown that is not from a vault.
class MarkdownText extends StatelessWidget {
  const MarkdownText({super.key, required this.markdown});
  final String markdown;

  @override
  Widget build(BuildContext context) {
    final blocks = parseMarkdown(text: markdown);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      mainAxisSize: MainAxisSize.min,
      children: [for (final b in blocks) MarkdownBlock(block: b, dense: true)],
    );
  }
}

/// One Markdown block. [marks] maps span index to match starts for Find.
class MarkdownBlock extends StatelessWidget {
  const MarkdownBlock({
    super.key,
    required this.block,
    this.marks,
    this.markLength = 0,
    this.current,
    this.dense = false,
  });

  final MdBlock block;
  final Map<int, List<int>>? marks;
  final int markLength;

  /// (span, start) of the current match, when it is in this block.
  final (int, int)? current;
  final bool dense;

  TextSpan _spans(TextStyle base, ColorScheme scheme) {
    final b = block;
    return TextSpan(
      style: base,
      children: [
        for (var i = 0; i < b.spans.length; i++)
          _span(i, b.spans[i], scheme, b.kind != MdBlockKind.code),
      ],
    );
  }

  InlineSpan _span(int i, MdSpan s, ColorScheme scheme, bool inlineCode) {
    final style = TextStyle(
      fontWeight: s.bold ? FontWeight.bold : null,
      fontStyle: s.italic ? FontStyle.italic : null,
      decoration: s.strike
          ? TextDecoration.lineThrough
          : s.link != null
          ? TextDecoration.underline
          : null,
      color: s.link != null ? scheme.primary : null,
      fontFamily: s.code ? 'monospace' : null,
      fontFamilyFallback: s.code
          ? const ['Menlo', 'Consolas', 'Courier New']
          : null,
      backgroundColor: s.code && inlineCode
          ? scheme.surfaceContainerHighest
          : null,
    );
    final text = mdSpanText(s);
    final starts = marks?[i];
    if (starts == null || markLength == 0) {
      return TextSpan(text: text, style: style);
    }
    return TextSpan(
      children: highlightSpans(
        text,
        starts,
        markLength,
        style,
        scheme,
        currentAt: current?.$1 == i ? current!.$2 : null,
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final b = block;
    final t = Theme.of(context).textTheme;
    final scheme = Theme.of(context).colorScheme;
    final body = (dense ? t.bodyMedium! : t.bodyLarge!).copyWith(height: 1.5);
    switch (b.kind) {
      case MdBlockKind.heading:
        final style = dense
            ? t.titleSmall!
            : switch (b.level) {
                1 => t.headlineMedium,
                2 => t.headlineSmall,
                3 => t.titleLarge,
                _ => t.titleMedium,
              }!;
        return Padding(
          padding: EdgeInsets.only(top: dense ? 8 : 16, bottom: dense ? 4 : 8),
          child: Text.rich(_spans(style, scheme)),
        );
      case MdBlockKind.paragraph:
        return Padding(
          padding: EdgeInsets.only(bottom: dense ? 8 : 12),
          child: Text.rich(_spans(body, scheme)),
        );
      case MdBlockKind.bullet:
      case MdBlockKind.numbered:
      case MdBlockKind.task:
        final marker = switch (b.kind) {
          MdBlockKind.numbered => Text('${b.number}.', style: body),
          MdBlockKind.task => Icon(
            b.flag ? Icons.check_box : Icons.check_box_outline_blank,
            size: 20,
            color: scheme.primary,
          ),
          _ => Text(b.level > 1 ? '◦' : '•', style: body),
        };
        return Padding(
          padding: EdgeInsets.only(left: 20.0 * (b.level - 1), bottom: 4),
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              SizedBox(width: 28, child: marker),
              Expanded(child: Text.rich(_spans(body, scheme))),
            ],
          ),
        );
      case MdBlockKind.quote:
        return Container(
          margin: const EdgeInsets.only(bottom: 12),
          padding: const EdgeInsets.only(left: 12),
          decoration: BoxDecoration(
            border: Border(left: BorderSide(color: scheme.outline, width: 3)),
          ),
          child: Text.rich(
            _spans(body.copyWith(color: scheme.onSurfaceVariant), scheme),
          ),
        );
      case MdBlockKind.code:
        return Container(
          width: double.infinity,
          margin: const EdgeInsets.only(bottom: 12),
          padding: const EdgeInsets.all(12),
          decoration: BoxDecoration(
            color: scheme.surfaceContainerHighest,
            borderRadius: BorderRadius.circular(8),
          ),
          child: SingleChildScrollView(
            scrollDirection: Axis.horizontal,
            child: Text.rich(
              _spans(
                const TextStyle(
                  fontFamily: 'monospace',
                  fontFamilyFallback: ['Menlo', 'Consolas', 'Courier New'],
                  fontSize: 13,
                ),
                scheme,
              ),
            ),
          ),
        );
      case MdBlockKind.rule:
        return Divider(height: dense ? 16 : 32);
      case MdBlockKind.tableRow:
        return Container(
          padding: const EdgeInsets.symmetric(vertical: 6, horizontal: 8),
          decoration: BoxDecoration(
            border: Border(bottom: BorderSide(color: scheme.outlineVariant)),
            color: b.flag ? scheme.surfaceContainerHighest : null,
          ),
          child: Text.rich(
            _spans(
              body.copyWith(fontWeight: b.flag ? FontWeight.bold : null),
              scheme,
            ),
          ),
        );
    }
  }
}

// ---- editing -------------------------------------------------------------

/// Edits a text file; Save writes it back into the vault (as a new
/// encrypted copy, removing the old one) or rewrites the package.
class TextEditorPage extends StatefulWidget {
  const TextEditorPage({
    super.key,
    required this.name,
    required this.initial,
    required this.onSave,
    this.place = 'vault',
  });

  final String place;
  final String name;
  final String initial;
  final Future<void> Function(String text) onSave;

  @override
  State<TextEditorPage> createState() => _TextEditorPageState();
}

class _TextEditorPageState extends State<TextEditorPage> {
  late final _text = TextEditingController(text: widget.initial);
  bool _saving = false;

  bool get _changed => _text.text != widget.initial;

  @override
  void dispose() {
    _text.dispose();
    super.dispose();
  }

  Future<void> _save() async {
    setState(() => _saving = true);
    try {
      await widget.onSave(_text.text);
      if (mounted) Navigator.of(context).pop(true);
    } catch (e) {
      if (mounted) {
        setState(() => _saving = false);
        showMessage(context, describeError(e), error: true);
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return PopScope(
      canPop: !_changed || _saving,
      onPopInvokedWithResult: (didPop, _) async {
        if (didPop) return;
        final discard = await confirm(
          context,
          title: 'Discard changes?',
          message: 'Your edits to ${widget.name} are not saved.',
          action: 'Discard',
          destructive: true,
        );
        if (discard && context.mounted) Navigator.of(context).pop(false);
      },
      child: Scaffold(
        appBar: AppBar(
          title: Text('Edit ${widget.name}', overflow: TextOverflow.ellipsis),
          actions: [
            ListenableBuilder(
              listenable: _text,
              builder: (context, _) => TextButton.icon(
                key: const Key('editor-save'),
                onPressed: _changed && !_saving ? _save : null,
                icon: _saving
                    ? const SizedBox.square(
                        dimension: 16,
                        child: CircularProgressIndicator(strokeWidth: 2),
                      )
                    : const Icon(Icons.save_outlined),
                label: const Text('Save'),
              ),
            ),
          ],
        ),
        body: Padding(
          padding: const EdgeInsets.all(8),
          child: TextField(
            key: const Key('editor-field'),
            controller: _text,
            expands: true,
            maxLines: null,
            textAlignVertical: TextAlignVertical.top,
            keyboardType: TextInputType.multiline,
            // Keep the keyboard from learning words from private files.
            autocorrect: false,
            enableSuggestions: false,
            enableIMEPersonalizedLearning: false,
            contextMenuBuilder: (context, state) =>
                AdaptiveTextSelectionToolbar.buttonItems(
                  anchors: state.contextMenuAnchors,
                  buttonItems: SecureClipboard.guard(
                    context,
                    state.contextMenuButtonItems,
                  ),
                ),
            style: TextStyle(
              fontFamily: 'monospace',
              fontFamilyFallback: const ['Menlo', 'Consolas', 'Courier New'],
              fontSize: 14,
              height: 1.4,
              color: scheme.onSurface,
            ),
            decoration: const InputDecoration(border: InputBorder.none),
          ),
        ),
      ),
    );
  }
}

// ---- audio ---------------------------------------------------------------

String _clock(int ms) {
  final s = ms ~/ 1000;
  final h = s ~/ 3600;
  final m = (s % 3600) ~/ 60;
  final sec = (s % 60).toString().padLeft(2, '0');
  return h > 0 ? '$h:${m.toString().padLeft(2, '0')}:$sec' : '$m:$sec';
}

/// Plays audio inside Atropos: Rust decodes it in memory and sends it to
/// the speakers itself, so no other app or file ever gets the sound.
class AudioView extends StatefulWidget {
  const AudioView({super.key, required this.viewer, required this.name});
  final FileViewer viewer;
  final String name;

  @override
  State<AudioView> createState() => _AudioViewState();
}

class _AudioViewState extends State<AudioView> {
  AudioInfo? _info;
  AudioState? _state;
  Object? _error;
  double? _dragging;
  Timer? _poll;

  @override
  void initState() {
    super.initState();
    widget.viewer.audioInfo().then(
      (i) {
        if (mounted) setState(() => _info = i);
      },
      onError: (Object e) {
        if (mounted) setState(() => _error = e);
      },
    );
    _poll = Timer.periodic(const Duration(milliseconds: 250), (_) => _read());
  }

  void _read() {
    if (!mounted) return;
    try {
      final s = widget.viewer.audioState();
      setState(() => _state = s);
    } catch (_) {
      _poll?.cancel();
    }
  }

  @override
  void dispose() {
    _poll?.cancel();
    // Closing the viewer stops the sound; pause first so it stops at once.
    widget.viewer.audioPause().ignore();
    super.dispose();
  }

  Future<void> _run(Future<void> Function() op) async {
    try {
      await op();
      _read();
    } catch (e) {
      if (mounted) showMessage(context, describeError(e), error: true);
    }
  }

  @override
  Widget build(BuildContext context) {
    if (_error != null) return _Failed(_error!);
    final info = _info;
    if (info == null) return const Center(child: CircularProgressIndicator());
    final st = _state;
    final t = Theme.of(context).textTheme;
    final scheme = Theme.of(context).colorScheme;
    final duration = info.durationMs;
    final pos = (_dragging ?? (st?.positionMs ?? 0).toDouble()).clamp(
      0.0,
      math.max(duration, 1).toDouble(),
    );
    final playing = st?.playing ?? false;
    final subtitle = [
      info.artist,
      info.album,
    ].where((x) => x.isNotEmpty).join(' · ');
    return Center(
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: 520),
        child: ListView(
          key: const Key('viewer-audio'),
          shrinkWrap: true,
          padding: const EdgeInsets.all(24),
          children: [
            Icon(Icons.audiotrack, size: 96, color: scheme.primary),
            const SizedBox(height: 16),
            Text(
              info.title.isNotEmpty ? info.title : widget.name,
              textAlign: TextAlign.center,
              style: t.titleLarge,
            ),
            if (subtitle.isNotEmpty)
              Text(subtitle, textAlign: TextAlign.center, style: t.bodyMedium),
            const SizedBox(height: 24),
            Slider(
              key: const Key('audio-position'),
              value: pos,
              max: math.max(duration, 1).toDouble(),
              onChanged: duration > 0
                  ? (v) => setState(() => _dragging = v)
                  : null,
              onChangeEnd: (v) {
                setState(() => _dragging = null);
                _run(() => widget.viewer.audioSeek(positionMs: v.round()));
              },
            ),
            Padding(
              padding: const EdgeInsets.symmetric(horizontal: 24),
              child: Row(
                mainAxisAlignment: MainAxisAlignment.spaceBetween,
                children: [
                  Text(_clock(pos.round()), style: t.bodySmall),
                  Text(
                    duration > 0 ? _clock(duration) : '',
                    style: t.bodySmall,
                  ),
                ],
              ),
            ),
            const SizedBox(height: 12),
            Row(
              mainAxisAlignment: MainAxisAlignment.center,
              children: [
                IconButton(
                  tooltip: 'Back 10 seconds',
                  iconSize: 32,
                  onPressed: () => _run(
                    () => widget.viewer.audioSeek(
                      positionMs: math.max(0, pos.round() - 10000),
                    ),
                  ),
                  icon: const Icon(Icons.replay_10),
                ),
                const SizedBox(width: 16),
                IconButton.filled(
                  key: const Key('audio-play'),
                  tooltip: playing ? 'Pause' : 'Play',
                  iconSize: 40,
                  onPressed: () => _run(
                    playing
                        ? widget.viewer.audioPause
                        : widget.viewer.audioPlay,
                  ),
                  icon: Icon(playing ? Icons.pause : Icons.play_arrow),
                ),
                const SizedBox(width: 16),
                IconButton(
                  tooltip: 'Forward 10 seconds',
                  iconSize: 32,
                  onPressed: () => _run(
                    () => widget.viewer.audioSeek(
                      positionMs: pos.round() + 10000,
                    ),
                  ),
                  icon: const Icon(Icons.forward_10),
                ),
              ],
            ),
            if (st?.error != null) ...[
              const SizedBox(height: 12),
              Text(
                'Sound output problem: ${st!.error}',
                textAlign: TextAlign.center,
                style: TextStyle(color: scheme.error),
              ),
            ],
          ],
        ),
      ),
    );
  }
}
