import 'dart:math' as math;
import 'dart:ui' as ui;

import 'package:flutter/material.dart';

import '../core/errors.dart';
import '../src/rust/api/viewer.dart';
import 'viewer_page.dart' show pictureToImage;
import 'widgets/feedback.dart';

/// Rotates, flips, crops and resizes a picture. The picture never leaves
/// Rust: each change is applied there and only a preview comes back. Save
/// writes the result over the file (or as a copy beside it), re-encoded in
/// the same format. Re-encoding drops the photo's metadata, such as where
/// it was taken.
class ImageEditorPage extends StatefulWidget {
  const ImageEditorPage({
    super.key,
    required this.viewer,
    required this.name,
    required this.onSave,
    this.place = 'vault',
    this.canSaveCopy = false,
  });

  final FileViewer viewer;
  final String name;
  final String place;
  final bool canSaveCopy;
  final Future<void> Function(List<ImageEdit> edits, bool asCopy) onSave;

  @override
  State<ImageEditorPage> createState() => _ImageEditorPageState();
}

const _resizeChoices = [4096, 3000, 2048, 1600, 1280, 1024, 800, 640];

class _ImageEditorPageState extends State<ImageEditorPage> {
  final List<ImageEdit> _ops = [];
  ui.Image? _preview;
  Size? _original;
  Object? _error;
  bool _animated = false;
  bool _saving = false;
  Size? _box;
  int _ticket = 0;

  bool _cropping = false;
  Rect _crop = const Rect.fromLTRB(0, 0, 1, 1);

  @override
  void initState() {
    super.initState();
    _start();
  }

  Future<void> _start() async {
    try {
      final s = await widget.viewer.imageSize();
      final d = await widget.viewer.frameDelays();
      if (!mounted) return;
      setState(() {
        _original = Size(s.width, s.height);
        _animated = d.length > 1;
      });
    } catch (e) {
      if (mounted) setState(() => _error = e);
    }
  }

  @override
  void dispose() {
    _preview?.dispose();
    super.dispose();
  }

  /// Size of the edited picture, in pixels.
  Size get _dims {
    var s = _original ?? Size.zero;
    for (final op in _ops) {
      s = switch (op) {
        ImageEdit_RotateLeft() || ImageEdit_RotateRight() => s.flipped,
        ImageEdit_FlipHorizontal() || ImageEdit_FlipVertical() => s,
        ImageEdit_Crop(:final left, :final top, :final right, :final bottom) =>
          Size(
            math.max(1, ((right - left) * s.width).roundToDouble()),
            math.max(1, ((bottom - top) * s.height).roundToDouble()),
          ),
        ImageEdit_Resize(:final maxSide) => () {
          final long = math.max(s.width, s.height);
          if (long <= maxSide) return s;
          final f = maxSide / long;
          return Size(
            math.max(1, (s.width * f).roundToDouble()),
            math.max(1, (s.height * f).roundToDouble()),
          );
        }(),
      };
    }
    return s;
  }

  Future<void> _render() async {
    final box = _box;
    if (box == null) return;
    final dpr = MediaQuery.devicePixelRatioOf(context);
    final ticket = ++_ticket;
    try {
      final p = await widget.viewer.renderEdited(
        edits: List.of(_ops),
        maxWidth: (box.width * dpr).ceil(),
        maxHeight: (box.height * dpr).ceil(),
      );
      final img = await pictureToImage(p);
      if (!mounted || ticket != _ticket) {
        img.dispose();
        return;
      }
      setState(() {
        _preview?.dispose();
        _preview = img;
      });
    } catch (e) {
      if (mounted) showMessage(context, describeError(e), error: true);
    }
  }

  void _add(ImageEdit op) {
    setState(() => _ops.add(op));
    _render();
  }

  void _undo() {
    if (_ops.isEmpty) return;
    setState(() => _ops.removeLast());
    _render();
  }

  void _reset() {
    setState(_ops.clear);
    _render();
  }

  void _applyCrop() {
    final c = _crop;
    setState(() {
      _cropping = false;
      _crop = const Rect.fromLTRB(0, 0, 1, 1);
    });
    if (c.width > 0.999 && c.height > 0.999) return;
    _add(
      ImageEdit.crop(
        left: c.left,
        top: c.top,
        right: c.right,
        bottom: c.bottom,
      ),
    );
  }

  Future<void> _resize() async {
    final d = _dims;
    final long = math.max(d.width, d.height).round();
    final pick = await showDialog<int>(
      context: context,
      builder: (ctx) => SimpleDialog(
        title: const Text('Resize'),
        children: [
          Padding(
            padding: const EdgeInsets.fromLTRB(24, 0, 24, 8),
            child: Text(
              'Now ${d.width.round()} × ${d.height.round()}. Pick the longest '
              'side; the shape stays the same.',
            ),
          ),
          for (final c in _resizeChoices.where((c) => c < long))
            ListTile(
              key: Key('resize-$c'),
              title: Text('$c pixels'),
              subtitle: Text(_scaled(d, c)),
              onTap: () => Navigator.pop(ctx, c),
            ),
          if (!_resizeChoices.any((c) => c < long))
            const Padding(
              padding: EdgeInsets.all(24),
              child: Text('This picture is already small.'),
            ),
        ],
      ),
    );
    if (pick != null) _add(ImageEdit.resize(maxSide: pick));
  }

  static String _scaled(Size d, int side) {
    final f = side / math.max(d.width, d.height);
    return '${(d.width * f).round()} × ${(d.height * f).round()}';
  }

  Future<void> _save(bool asCopy) async {
    setState(() => _saving = true);
    try {
      await widget.onSave(List.of(_ops), asCopy);
      if (!mounted) return;
      showMessage(
        context,
        asCopy
            ? 'Saved as a copy in the ${widget.place}.'
            : 'Saved into the ${widget.place}.',
      );
      Navigator.of(context).pop(true);
    } catch (e) {
      if (mounted) {
        setState(() => _saving = false);
        showMessage(context, describeError(e), error: true);
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    final changed = _ops.isNotEmpty;
    final scheme = Theme.of(context).colorScheme;
    final d = _dims;
    return PopScope(
      canPop: !changed || _saving,
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
            if (_saving)
              const Padding(
                padding: EdgeInsets.all(16),
                child: SizedBox.square(
                  dimension: 20,
                  child: CircularProgressIndicator(strokeWidth: 2),
                ),
              )
            else if (widget.canSaveCopy)
              PopupMenuButton<bool>(
                key: const Key('picture-save'),
                enabled: changed && !_cropping,
                tooltip: 'Save',
                icon: const Icon(Icons.save_outlined),
                onSelected: _save,
                itemBuilder: (_) => const [
                  PopupMenuItem(
                    key: Key('picture-save-replace'),
                    value: false,
                    child: Text('Save (replace the picture)'),
                  ),
                  PopupMenuItem(
                    key: Key('picture-save-copy'),
                    value: true,
                    child: Text('Save as a copy'),
                  ),
                ],
              )
            else
              TextButton.icon(
                key: const Key('picture-save'),
                onPressed: changed && !_cropping ? () => _save(false) : null,
                icon: const Icon(Icons.save_outlined),
                label: const Text('Save'),
              ),
          ],
        ),
        body: _error != null
            ? Center(child: Text(describeError(_error!)))
            : _animated
            ? const Center(
                child: Padding(
                  padding: EdgeInsets.all(24),
                  child: Text(
                    'Animated pictures cannot be edited yet.',
                    textAlign: TextAlign.center,
                  ),
                ),
              )
            : Column(
                children: [
                  Expanded(
                    child: ColoredBox(
                      color: Colors.black,
                      child: LayoutBuilder(
                        builder: (context, c) {
                          final box = Size(c.maxWidth - 32, c.maxHeight - 32);
                          if (_box != box && _original != null) {
                            _box = box;
                            WidgetsBinding.instance.addPostFrameCallback((_) {
                              if (mounted) _render();
                            });
                          }
                          final img = _preview;
                          if (img == null) {
                            return const Center(
                              child: CircularProgressIndicator(),
                            );
                          }
                          final fit = applyBoxFit(
                            BoxFit.contain,
                            Size(img.width.toDouble(), img.height.toDouble()),
                            box,
                          ).destination;
                          final shown = Alignment.center.inscribe(
                            fit,
                            Offset(16, 16) & box,
                          );
                          return Stack(
                            children: [
                              Positioned.fromRect(
                                rect: shown,
                                child: RawImage(
                                  key: const Key('picture-preview'),
                                  image: img,
                                  fit: BoxFit.fill,
                                ),
                              ),
                              if (_cropping)
                                _CropOverlay(
                                  area: shown,
                                  crop: _crop,
                                  onChanged: (r) => setState(() => _crop = r),
                                ),
                            ],
                          );
                        },
                      ),
                    ),
                  ),
                  Padding(
                    padding: const EdgeInsets.only(top: 6),
                    child: Text(
                      '${d.width.round()} × ${d.height.round()} pixels'
                      '${changed ? ' · saving removes camera details and location' : ''}',
                      key: const Key('picture-size'),
                      style: Theme.of(context).textTheme.bodySmall,
                    ),
                  ),
                  SafeArea(
                    top: false,
                    child: _cropping
                        ? Padding(
                            padding: const EdgeInsets.all(8),
                            child: Row(
                              mainAxisAlignment: MainAxisAlignment.end,
                              children: [
                                TextButton(
                                  onPressed: () => setState(() {
                                    _cropping = false;
                                    _crop = const Rect.fromLTRB(0, 0, 1, 1);
                                  }),
                                  child: const Text('Cancel'),
                                ),
                                const SizedBox(width: 8),
                                FilledButton(
                                  key: const Key('crop-apply'),
                                  onPressed: _applyCrop,
                                  child: const Text('Crop'),
                                ),
                              ],
                            ),
                          )
                        : SingleChildScrollView(
                            scrollDirection: Axis.horizontal,
                            padding: const EdgeInsets.symmetric(
                              horizontal: 8,
                              vertical: 4,
                            ),
                            child: Row(
                              children: [
                                _Tool(
                                  key: const Key('edit-rotate-left'),
                                  icon: Icons.rotate_left,
                                  label: 'Left',
                                  onTap: () =>
                                      _add(const ImageEdit.rotateLeft()),
                                ),
                                _Tool(
                                  key: const Key('edit-rotate-right'),
                                  icon: Icons.rotate_right,
                                  label: 'Right',
                                  onTap: () =>
                                      _add(const ImageEdit.rotateRight()),
                                ),
                                _Tool(
                                  key: const Key('edit-flip-h'),
                                  icon: Icons.flip,
                                  label: 'Flip',
                                  onTap: () =>
                                      _add(const ImageEdit.flipHorizontal()),
                                ),
                                _Tool(
                                  key: const Key('edit-flip-v'),
                                  icon: Icons.flip,
                                  turn: true,
                                  label: 'Flip up',
                                  onTap: () =>
                                      _add(const ImageEdit.flipVertical()),
                                ),
                                _Tool(
                                  key: const Key('edit-crop'),
                                  icon: Icons.crop,
                                  label: 'Crop',
                                  onTap: () => setState(() => _cropping = true),
                                ),
                                _Tool(
                                  key: const Key('edit-resize'),
                                  icon: Icons.photo_size_select_large,
                                  label: 'Resize',
                                  onTap: _resize,
                                ),
                                _Tool(
                                  key: const Key('edit-undo'),
                                  icon: Icons.undo,
                                  label: 'Undo',
                                  onTap: changed ? _undo : null,
                                ),
                                _Tool(
                                  icon: Icons.restart_alt,
                                  label: 'Reset',
                                  onTap: changed ? _reset : null,
                                  color: scheme.error,
                                ),
                              ],
                            ),
                          ),
                  ),
                ],
              ),
      ),
    );
  }
}

class _Tool extends StatelessWidget {
  const _Tool({
    super.key,
    required this.icon,
    required this.label,
    required this.onTap,
    this.turn = false,
    this.color,
  });

  final IconData icon;
  final String label;
  final VoidCallback? onTap;
  final bool turn;
  final Color? color;

  @override
  Widget build(BuildContext context) {
    final i = Icon(icon, color: onTap == null ? null : color);
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 2),
      child: TextButton(
        onPressed: onTap,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            turn ? RotatedBox(quarterTurns: 1, child: i) : i,
            const SizedBox(height: 2),
            Text(label),
          ],
        ),
      ),
    );
  }
}

/// A crop frame over the preview: drag a corner to resize it, or the
/// middle to move it. [crop] is in fractions of the picture.
class _CropOverlay extends StatelessWidget {
  const _CropOverlay({
    required this.area,
    required this.crop,
    required this.onChanged,
  });

  final Rect area;
  final Rect crop;
  final ValueChanged<Rect> onChanged;

  static const _min = 0.05;
  static const _handle = 28.0;

  Rect get _frame => Rect.fromLTRB(
    area.left + crop.left * area.width,
    area.top + crop.top * area.height,
    area.left + crop.right * area.width,
    area.top + crop.bottom * area.height,
  );

  void _drag(
    Offset delta, {
    bool l = false,
    bool t = false,
    bool r = false,
    bool b = false,
  }) {
    final dx = delta.dx / area.width;
    final dy = delta.dy / area.height;
    var c = crop;
    if (!l && !t && !r && !b) {
      // Move the whole frame, keeping it inside the picture.
      final mx = dx.clamp(-c.left, 1 - c.right);
      final my = dy.clamp(-c.top, 1 - c.bottom);
      onChanged(c.shift(Offset(mx, my)));
      return;
    }
    c = Rect.fromLTRB(
      l ? (c.left + dx).clamp(0.0, c.right - _min) : c.left,
      t ? (c.top + dy).clamp(0.0, c.bottom - _min) : c.top,
      r ? (c.right + dx).clamp(c.left + _min, 1.0) : c.right,
      b ? (c.bottom + dy).clamp(c.top + _min, 1.0) : c.bottom,
    );
    onChanged(c);
  }

  Widget _corner(Offset at, {required bool l, required bool t}) => Positioned(
    left: at.dx - _handle / 2,
    top: at.dy - _handle / 2,
    width: _handle,
    height: _handle,
    child: GestureDetector(
      key: Key('crop-${t ? 'top' : 'bottom'}-${l ? 'left' : 'right'}'),
      behavior: HitTestBehavior.opaque,
      onPanUpdate: (d) => _drag(d.delta, l: l, t: t, r: !l, b: !t),
      child: Center(
        child: Container(
          width: 16,
          height: 16,
          decoration: BoxDecoration(
            color: Colors.white,
            shape: BoxShape.circle,
            border: Border.all(color: Colors.black54),
          ),
        ),
      ),
    ),
  );

  @override
  Widget build(BuildContext context) {
    final f = _frame;
    return Positioned.fill(
      child: Stack(
        children: [
          Positioned.fill(
            child: IgnorePointer(
              child: CustomPaint(painter: _ShadePainter(area, f)),
            ),
          ),
          Positioned.fromRect(
            rect: f,
            child: GestureDetector(
              key: const Key('crop-frame'),
              behavior: HitTestBehavior.opaque,
              onPanUpdate: (d) => _drag(d.delta),
              child: Container(
                decoration: BoxDecoration(
                  border: Border.all(color: Colors.white, width: 2),
                ),
              ),
            ),
          ),
          _corner(f.topLeft, l: true, t: true),
          _corner(f.topRight, l: false, t: true),
          _corner(f.bottomLeft, l: true, t: false),
          _corner(f.bottomRight, l: false, t: false),
        ],
      ),
    );
  }
}

class _ShadePainter extends CustomPainter {
  _ShadePainter(this.area, this.frame);
  final Rect area;
  final Rect frame;

  @override
  void paint(Canvas canvas, Size size) {
    final p = Path()
      ..fillType = PathFillType.evenOdd
      ..addRect(area)
      ..addRect(frame);
    canvas.drawPath(p, Paint()..color = Colors.black.withValues(alpha: 0.55));
  }

  @override
  bool shouldRepaint(_ShadePainter old) =>
      old.area != area || old.frame != frame;
}
