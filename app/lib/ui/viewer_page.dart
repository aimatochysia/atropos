import 'dart:async';
import 'dart:math' as math;
import 'dart:ui' as ui;

import 'package:flutter/material.dart';

import '../core/errors.dart';
import '../core/format.dart';
import '../core/secure_clipboard.dart';
import '../src/rust/api/vault.dart';
import '../src/rust/api/viewer.dart';
import '../state/app_controller.dart';
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

/// Shows one file from a vault or a package. The plaintext stays in Rust;
/// it is wiped when this page closes or the app locks.
class ViewerPage extends StatefulWidget {
  const ViewerPage({
    super.key,
    required this.entry,
    required this.open,
    required this.isClosed,
    this.onExport,
    this.onSave,
  });

  /// A file in an unlocked vault: viewable, editable and exportable.
  ViewerPage.vault({
    super.key,
    required VaultSession session,
    required this.entry,
    required Future<void> Function() this.onExport,
  }) : open = (() => session.openViewer(id: entry.id)),
       isClosed = (() => session.isLocked()),
       onSave = ((text) => session.saveText(id: entry.id, text: text));

  final EntryInfo entry;
  final Future<FileViewer> Function() open;

  /// True once the vault or package the file came from has closed.
  final bool Function() isClosed;

  /// Runs the usual export flow, with its confirmation. None for packages.
  final Future<void> Function()? onExport;

  /// Saves edited text back. None where files are read-only.
  final Future<void> Function(String text)? onSave;

  @override
  State<ViewerPage> createState() => _ViewerPageState();
}

class _ViewerPageState extends State<ViewerPage> {
  FileViewer? _viewer;
  String? _error;
  int _version = 0;

  @override
  void initState() {
    super.initState();
    _open();
  }

  Future<void> _open() async {
    try {
      final v = await widget.open();
      if (!mounted) {
        v.close();
        return;
      }
      setState(() {
        _viewer?.close();
        _viewer = v;
        _version++;
      });
    } catch (e) {
      if (mounted) setState(() => _error = describeError(e));
    }
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    AppScope.of(context);
    if (widget.isClosed()) _viewer?.close();
  }

  @override
  void dispose() {
    _viewer?.close();
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
    final saved = await Navigator.of(context).push<bool>(
      MaterialPageRoute(
        builder: (_) => TextEditorPage(
          name: widget.entry.name,
          initial: text,
          onSave: widget.onSave!,
        ),
      ),
    );
    if (saved == true && mounted) {
      showMessage(context, 'Saved into the vault.');
      await _open();
    }
  }

  @override
  Widget build(BuildContext context) {
    final v = _viewer;
    final canEdit =
        widget.onSave != null &&
        v != null &&
        (v.kind == PreviewKind.text || v.kind == PreviewKind.markdown);
    return Scaffold(
      appBar: AppBar(
        title: Text(widget.entry.name, overflow: TextOverflow.ellipsis),
        actions: [
          if (canEdit)
            IconButton(
              key: const Key('viewer-edit'),
              tooltip: 'Edit',
              icon: const Icon(Icons.edit_outlined),
              onPressed: () => _edit(v),
            ),
          if (widget.onExport != null)
            IconButton(
              key: const Key('viewer-export'),
              tooltip: 'Export unencrypted…',
              icon: const Icon(Icons.file_download_outlined),
              onPressed: widget.onExport,
            ),
        ],
      ),
      body: _error != null
          ? FileDetails(
              entry: widget.entry,
              message: _error,
              onExport: widget.onExport,
            )
          : v == null
          ? const Center(child: CircularProgressIndicator())
          : KeyedSubtree(
              key: ValueKey(_version),
              child: switch (v.kind) {
                PreviewKind.image => ImageView(viewer: v),
                PreviewKind.pdf => PdfView(viewer: v),
                PreviewKind.markdown => MarkdownView(viewer: v),
                PreviewKind.text => TextFileView(viewer: v),
                PreviewKind.audio => AudioView(
                  viewer: v,
                  name: widget.entry.name,
                ),
                PreviewKind.none => FileDetails(
                  entry: widget.entry,
                  onExport: widget.onExport,
                ),
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
    this.message,
  });

  final EntryInfo entry;
  final Future<void> Function()? onExport;
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
                  ? 'Atropos keeps this file encrypted but cannot show it. '
                        'Export it to open it in another app.'
                  : 'Atropos cannot show this kind of file.'),
          key: const Key('viewer-message'),
          textAlign: TextAlign.center,
        ),
        if (onExport != null) ...[
          const SizedBox(height: 16),
          Center(
            child: OutlinedButton.icon(
              onPressed: onExport,
              icon: const Icon(Icons.file_download_outlined),
              label: const Text('Export unencrypted…'),
            ),
          ),
        ],
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
  const ImageView({super.key, required this.viewer});
  final FileViewer viewer;

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

  void _onZoom() {
    if (_delays != null) return;
    if (!_sharp && _zoom.value.getMaxScaleOnAxis() > 1.8 && _box != null) {
      _sharp = true;
      _render(_box!, factor: 6);
    }
  }

  Future<void> _render(Size box, {double factor = 2}) async {
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
                child: RawImage(
                  key: const Key('viewer-image'),
                  image: img,
                  fit: BoxFit.contain,
                  filterQuality: FilterQuality.medium,
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

/// Pages one under another. Only pages near the screen keep a rendering;
/// tapping a page opens it alone with zoom.
class PdfView extends StatefulWidget {
  const PdfView({super.key, required this.viewer});
  final FileViewer viewer;

  @override
  State<PdfView> createState() => _PdfViewState();
}

class _PdfViewState extends State<PdfView> {
  List<PictureSize>? _pages;
  Object? _error;
  final Map<int, ui.Image> _images = {};
  final Set<int> _pending = {};
  int _width = 0;

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
  }

  @override
  void dispose() {
    for (final i in _images.values) {
      i.dispose();
    }
    super.dispose();
  }

  Future<void> _load(int index, int width) async {
    if (_pending.contains(index)) return;
    _pending.add(index);
    try {
      final p = await widget.viewer.renderPage(
        index: index,
        maxWidth: width,
        maxHeight: 4096,
      );
      final img = await pictureToImage(p);
      if (!mounted || width != _width) {
        img.dispose();
        return;
      }
      setState(() {
        _images.remove(index)?.dispose();
        _images[index] = img;
        // Keep memory in check: forget pages far from this one.
        final far = _images.keys.where((k) => (k - index).abs() > 4).toList();
        for (final k in far) {
          _images.remove(k)?.dispose();
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
        final cssWidth = math.min(c.maxWidth - 24, 1000.0);
        final width = (cssWidth * dpr).round();
        if (width != _width) {
          _width = width;
          for (final i in _images.values) {
            i.dispose();
          }
          _images.clear();
        }
        return ColoredBox(
          color: scheme.surfaceContainerHighest,
          child: ListView.builder(
            key: const Key('viewer-pdf'),
            padding: const EdgeInsets.symmetric(vertical: 12),
            itemCount: pages.length,
            itemBuilder: (context, i) {
              final size = pages[i];
              final img = _images[i];
              if (img == null) _load(i, width);
              return Center(
                child: Padding(
                  padding: const EdgeInsets.symmetric(vertical: 6),
                  child: GestureDetector(
                    onTap: () => Navigator.of(context).push(
                      MaterialPageRoute<void>(
                        builder: (_) => _PdfPageZoom(
                          viewer: widget.viewer,
                          index: i,
                          count: pages.length,
                          size: size,
                        ),
                      ),
                    ),
                    child: Container(
                      width: cssWidth,
                      height: cssWidth * size.height / size.width,
                      decoration: BoxDecoration(
                        color: Colors.white,
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
                ),
              );
            },
          ),
        );
      },
    );
  }
}

class _PdfPageZoom extends StatefulWidget {
  const _PdfPageZoom({
    required this.viewer,
    required this.index,
    required this.count,
    required this.size,
  });

  final FileViewer viewer;
  final int index;
  final int count;
  final PictureSize size;

  @override
  State<_PdfPageZoom> createState() => _PdfPageZoomState();
}

class _PdfPageZoomState extends State<_PdfPageZoom> {
  ui.Image? _image;
  Object? _error;
  bool _started = false;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (_started) return;
    _started = true;
    final size = MediaQuery.sizeOf(context);
    final dpr = MediaQuery.devicePixelRatioOf(context);
    final side = (math.max(size.width, size.height) * dpr * 2).round();
    widget.viewer
        .renderPage(index: widget.index, maxWidth: side, maxHeight: side)
        .then(pictureToImage)
        .then(
          (img) {
            if (!mounted) {
              img.dispose();
              return;
            }
            setState(() => _image = img);
          },
          onError: (Object e) {
            if (mounted) setState(() => _error = e);
          },
        );
  }

  @override
  void dispose() {
    _image?.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final img = _image;
    return Scaffold(
      appBar: AppBar(
        title: Text('Page ${widget.index + 1} of ${widget.count}'),
      ),
      body: _error != null
          ? _Failed(_error!)
          : img == null
          ? const Center(child: CircularProgressIndicator())
          : InteractiveViewer(
              maxScale: 8,
              child: SizedBox.expand(
                child: RawImage(image: img, fit: BoxFit.contain),
              ),
            ),
    );
  }
}

// ---- text ----------------------------------------------------------------

class TextFileView extends StatefulWidget {
  const TextFileView({super.key, required this.viewer});
  final FileViewer viewer;

  @override
  State<TextFileView> createState() => _TextFileViewState();
}

class _TextFileViewState extends State<TextFileView> {
  TextView? _text;
  Object? _error;
  bool _wrap = true;

  @override
  void initState() {
    super.initState();
    widget.viewer.text().then(
      (t) {
        if (mounted) setState(() => _text = t);
      },
      onError: (Object e) {
        if (mounted) setState(() => _error = e);
      },
    );
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
    final digits = '${t.lines.length}'.length;
    Widget list = ListView.builder(
      key: const Key('viewer-text'),
      padding: const EdgeInsets.fromLTRB(8, 8, 16, 24),
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
        return Row(
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
              Expanded(child: Text(t.lines[i], style: style))
            else
              Text(t.lines[i], style: style, softWrap: false),
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
                onPressed: () => setState(() => _wrap = !_wrap),
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
  const MarkdownView({super.key, required this.viewer});
  final FileViewer viewer;

  @override
  State<MarkdownView> createState() => _MarkdownViewState();
}

class _MarkdownViewState extends State<MarkdownView> {
  List<MdBlock>? _blocks;
  Object? _error;

  @override
  void initState() {
    super.initState();
    widget.viewer.markdown().then(
      (b) {
        if (mounted) setState(() => _blocks = b);
      },
      onError: (Object e) {
        if (mounted) setState(() => _error = e);
      },
    );
  }

  TextSpan _spans(MdBlock b, TextStyle base, ColorScheme scheme) => TextSpan(
    style: base,
    children: [
      for (final s in b.spans)
        TextSpan(
          text: s.link != null && s.link != s.text
              ? '${s.text} (${s.link})'
              : s.text,
          style: TextStyle(
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
            backgroundColor: s.code && b.kind != MdBlockKind.code
                ? scheme.surfaceContainerHighest
                : null,
          ),
        ),
    ],
  );

  Widget _block(BuildContext context, MdBlock b) {
    final t = Theme.of(context).textTheme;
    final scheme = Theme.of(context).colorScheme;
    final body = t.bodyLarge!.copyWith(height: 1.5);
    switch (b.kind) {
      case MdBlockKind.heading:
        final style = switch (b.level) {
          1 => t.headlineMedium,
          2 => t.headlineSmall,
          3 => t.titleLarge,
          _ => t.titleMedium,
        }!;
        return Padding(
          padding: const EdgeInsets.only(top: 16, bottom: 8),
          child: Text.rich(_spans(b, style, scheme)),
        );
      case MdBlockKind.paragraph:
        return Padding(
          padding: const EdgeInsets.only(bottom: 12),
          child: Text.rich(_spans(b, body, scheme)),
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
              Expanded(child: Text.rich(_spans(b, body, scheme))),
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
            _spans(b, body.copyWith(color: scheme.onSurfaceVariant), scheme),
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
                b,
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
        return const Divider(height: 32);
      case MdBlockKind.tableRow:
        return Container(
          padding: const EdgeInsets.symmetric(vertical: 6, horizontal: 8),
          decoration: BoxDecoration(
            border: Border(bottom: BorderSide(color: scheme.outlineVariant)),
            color: b.flag ? scheme.surfaceContainerHighest : null,
          ),
          child: Text.rich(
            _spans(
              b,
              body.copyWith(fontWeight: b.flag ? FontWeight.bold : null),
              scheme,
            ),
          ),
        );
    }
  }

  @override
  Widget build(BuildContext context) {
    if (_error != null) return _Failed(_error!);
    final blocks = _blocks;
    if (blocks == null) return const Center(child: CircularProgressIndicator());
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
            padding: const EdgeInsets.fromLTRB(20, 8, 20, 32),
            itemCount: blocks.length,
            itemBuilder: (context, i) => _block(context, blocks[i]),
          ),
        ),
      ),
    );
  }
}

// ---- editing -------------------------------------------------------------

/// Edits a text file; Save writes it back into the vault as a new
/// encrypted copy and removes the old one.
class TextEditorPage extends StatefulWidget {
  const TextEditorPage({
    super.key,
    required this.name,
    required this.initial,
    required this.onSave,
  });

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
