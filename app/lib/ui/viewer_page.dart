import 'dart:math' as math;
import 'dart:ui' as ui;

import 'package:flutter/material.dart';

import '../core/errors.dart';
import '../core/format.dart';
import '../src/rust/api/vault.dart';
import '../src/rust/api/viewer.dart';
import '../state/app_controller.dart';

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
  PreviewKind.none => Icons.insert_drive_file_outlined,
};

/// Shows one file from the vault. The plaintext stays in Rust; it is wiped
/// when this page closes or the vault locks.
class ViewerPage extends StatefulWidget {
  const ViewerPage({
    super.key,
    required this.session,
    required this.entry,
    required this.onExport,
  });

  final VaultSession session;
  final EntryInfo entry;

  /// Runs the usual export flow, with its confirmation.
  final Future<void> Function() onExport;

  @override
  State<ViewerPage> createState() => _ViewerPageState();
}

class _ViewerPageState extends State<ViewerPage> {
  FileViewer? _viewer;
  String? _error;

  @override
  void initState() {
    super.initState();
    _open();
  }

  Future<void> _open() async {
    try {
      final v = await widget.session.openViewer(id: widget.entry.id);
      if (!mounted) {
        v.close();
        return;
      }
      setState(() => _viewer = v);
    } catch (e) {
      if (mounted) setState(() => _error = describeError(e));
    }
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final app = AppScope.of(context);
    if (!app.isOpen(widget.session.path)) _viewer?.close();
  }

  @override
  void dispose() {
    _viewer?.close();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final v = _viewer;
    return Scaffold(
      appBar: AppBar(
        title: Text(widget.entry.name, overflow: TextOverflow.ellipsis),
        actions: [
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
          : switch (v.kind) {
              PreviewKind.image => ImageView(viewer: v),
              PreviewKind.pdf => PdfView(viewer: v),
              PreviewKind.markdown => MarkdownView(viewer: v),
              PreviewKind.text => TextFileView(viewer: v),
              PreviewKind.none => FileDetails(
                entry: widget.entry,
                onExport: widget.onExport,
              ),
            },
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
  final Future<void> Function() onExport;
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
              'Atropos keeps this file encrypted but cannot show it. Export '
                  'it to open it in another app.',
          key: const Key('viewer-message'),
          textAlign: TextAlign.center,
        ),
        const SizedBox(height: 16),
        Center(
          child: OutlinedButton.icon(
            onPressed: onExport,
            icon: const Icon(Icons.file_download_outlined),
            label: const Text('Export unencrypted…'),
          ),
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

  @override
  void initState() {
    super.initState();
    _zoom.addListener(_onZoom);
    widget.viewer.imageSize().then(
      (s) {
        if (mounted) setState(() => _full = Size(s.width, s.height));
      },
      onError: (Object e) {
        if (mounted) setState(() => _error = e);
      },
    );
  }

  void _onZoom() {
    if (!_sharp && _zoom.value.getMaxScaleOnAxis() > 1.8 && _box != null) {
      _sharp = true;
      _render(_box!, factor: 6);
    }
  }

  Future<void> _render(Size box, {double factor = 2}) async {
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

  @override
  void dispose() {
    _zoom.dispose();
    _image?.dispose();
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
        return InteractiveViewer(
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
    return Stack(
      children: [
        list,
        Positioned(
          right: 12,
          bottom: 12,
          child: FloatingActionButton.small(
            tooltip: _wrap ? 'Don\'t wrap lines' : 'Wrap lines',
            onPressed: () => setState(() => _wrap = !_wrap),
            child: Icon(_wrap ? Icons.wrap_text : Icons.notes),
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
        child: ListView.builder(
          key: const Key('viewer-markdown'),
          padding: const EdgeInsets.fromLTRB(20, 8, 20, 32),
          itemCount: blocks.length,
          itemBuilder: (context, i) => _block(context, blocks[i]),
        ),
      ),
    );
  }
}
