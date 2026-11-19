import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../src/rust/api/secure_input.dart';

export '../src/rust/api/secure_input.dart' show SecretInput, Strength;

/// Which secret field currently shows the Atropos keypad. Only one at a time.
final ValueNotifier<Object?> _activeKeypad = ValueNotifier(null);

/// A password or phrase entry whose characters go straight into Rust memory
/// ([SecretInput]) and are wiped there. Dart only ever knows how many
/// characters were typed.
///
/// With [secureKeyboard] the in-app keypad is used, so no system keyboard,
/// autocorrect, clipboard or keyboard app sees the secret. Without it a
/// normal obscured text field is used (the text then also passes through
/// Dart strings, which cannot be wiped).
class SecretField extends StatefulWidget {
  const SecretField({
    super.key,
    required this.input,
    required this.label,
    this.secureKeyboard = true,
    this.autofocus = false,
    this.helper,
    this.errorText,
    this.onChanged,
    this.onSubmitted,
    this.lettersOnly = false,
  });

  final SecretInput input;
  final String label;
  final bool secureKeyboard;
  final bool autofocus;
  final String? helper;
  final String? errorText;
  final VoidCallback? onChanged;
  final VoidCallback? onSubmitted;

  /// For recovery phrases: lowercase letters and spaces only.
  final bool lettersOnly;

  @override
  State<SecretField> createState() => _SecretFieldState();
}

class _SecretFieldState extends State<SecretField> {
  final _token = Object();
  final _text = TextEditingController();
  int _length = 0;

  @override
  void initState() {
    super.initState();
    _length = widget.input.len();
    _activeKeypad.addListener(_rebuild);
    if (widget.autofocus && widget.secureKeyboard) {
      WidgetsBinding.instance.addPostFrameCallback((_) => _open());
    }
  }

  @override
  void dispose() {
    _activeKeypad.removeListener(_rebuild);
    if (_activeKeypad.value == _token) _activeKeypad.value = null;
    _text.clear();
    _text.dispose();
    super.dispose();
  }

  void _rebuild() {
    if (mounted) setState(() {});
  }

  bool get _keypadOpen => _activeKeypad.value == _token;

  void _open() => _activeKeypad.value = _token;

  void _changed() {
    setState(() => _length = widget.input.len());
    widget.onChanged?.call();
  }

  void _type(int codepoint) {
    widget.input.push(codepoint: codepoint);
    _changed();
  }

  void _backspace() {
    widget.input.pop();
    _changed();
  }

  void _done() {
    _activeKeypad.value = null;
    widget.onSubmitted?.call();
  }

  /// System keyboard: mirror the field into Rust after every edit.
  void _systemChanged(String value) {
    widget.input.clear();
    for (final r in value.runes) {
      widget.input.push(codepoint: r);
    }
    _changed();
  }

  @override
  Widget build(BuildContext context) {
    if (!widget.secureKeyboard) {
      return TextField(
        controller: _text,
        autofocus: widget.autofocus,
        obscureText: true,
        enableSuggestions: false,
        autocorrect: false,
        enableIMEPersonalizedLearning: false,
        keyboardType: TextInputType.visiblePassword,
        decoration: InputDecoration(
          labelText: widget.label,
          helperText: widget.helper,
          errorText: widget.errorText,
          helperMaxLines: 3,
        ),
        onChanged: _systemChanged,
        onSubmitted: (_) => widget.onSubmitted?.call(),
      );
    }
    final scheme = Theme.of(context).colorScheme;
    final dots = '•' * _length.clamp(0, 64);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Semantics(
          label: widget.label,
          value: '$_length characters',
          button: true,
          child: InkWell(
            onTap: _keypadOpen ? null : _open,
            borderRadius: BorderRadius.circular(4),
            child: InputDecorator(
              isFocused: _keypadOpen,
              isEmpty: _length == 0,
              decoration: InputDecoration(
                labelText: widget.label,
                helperText: widget.helper,
                errorText: widget.errorText,
                helperMaxLines: 3,
                suffixIcon: _length > 0
                    ? IconButton(
                        tooltip: 'Clear',
                        icon: const Icon(Icons.clear),
                        onPressed: () {
                          widget.input.clear();
                          _changed();
                        },
                      )
                    : Icon(Icons.keyboard, color: scheme.outline),
              ),
              child: Text(
                dots,
                maxLines: 1,
                overflow: TextOverflow.clip,
                style: const TextStyle(letterSpacing: 2),
              ),
            ),
          ),
        ),
        AnimatedSize(
          duration: const Duration(milliseconds: 150),
          child: _keypadOpen
              ? Padding(
                  padding: const EdgeInsets.only(top: 8),
                  child: SecureKeypad(
                    onKey: _type,
                    onBackspace: _backspace,
                    onDone: _done,
                    lettersOnly: widget.lettersOnly,
                  ),
                )
              : const SizedBox(width: double.infinity),
        ),
      ],
    );
  }
}

/// The Atropos on-screen keyboard. Keys report Unicode code points; nothing
/// is ever assembled into a Dart string.
class SecureKeypad extends StatefulWidget {
  const SecureKeypad({
    super.key,
    required this.onKey,
    required this.onBackspace,
    required this.onDone,
    this.lettersOnly = false,
  });

  final ValueChanged<int> onKey;
  final VoidCallback onBackspace;
  final VoidCallback onDone;
  final bool lettersOnly;

  @override
  State<SecureKeypad> createState() => _SecureKeypadState();
}

enum _Shift { off, once, locked }

class _SecureKeypadState extends State<SecureKeypad> {
  static const _letters = ['1234567890', 'qwertyuiop', 'asdfghjkl', 'zxcvbnm'];
  static const _symbols = [
    '1234567890',
    '!@#\$%^&*()',
    '-_=+[]{}\\|',
    ";:'\",.<>/?",
    '`~',
  ];

  bool _symbolPage = false;
  _Shift _shift = _Shift.off;
  DateTime _lastShift = DateTime(0);

  void _press(String ch) {
    final upper = _shift != _Shift.off && !_symbolPage;
    widget.onKey((upper ? ch.toUpperCase() : ch).runes.first);
    if (_shift == _Shift.once) setState(() => _shift = _Shift.off);
  }

  void _toggleShift() {
    final now = DateTime.now();
    final doubleTap = now.difference(_lastShift).inMilliseconds < 350;
    _lastShift = now;
    setState(() {
      _shift = switch (_shift) {
        _Shift.off => doubleTap ? _Shift.locked : _Shift.once,
        _Shift.once => doubleTap ? _Shift.locked : _Shift.off,
        _Shift.locked => _Shift.off,
      };
    });
  }

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final upper = _shift != _Shift.off;
    Widget row(
      String chars, {
      List<Widget> lead = const [],
      List<Widget> trail = const [],
    }) {
      return Row(
        children: [
          ...lead,
          for (final c in chars.split(''))
            _Key(
              key: Key('key-$c'),
              label: upper && !_symbolPage ? c.toUpperCase() : c,
              onTap: () => _press(c),
            ),
          ...trail,
        ],
      );
    }

    final backspace = _Key(
      key: const Key('key-backspace'),
      icon: Icons.backspace_outlined,
      flex: 3,
      tooltip: 'Delete',
      onTap: widget.onBackspace,
      onLongPress: () {
        for (var i = 0; i < 64; i++) {
          widget.onBackspace();
        }
      },
    );
    final space = _Key(
      key: const Key('key-space'),
      label: 'space',
      flex: 10,
      onTap: () => widget.onKey(0x20),
    );
    final done = _Key(
      key: const Key('key-done'),
      icon: Icons.keyboard_return,
      flex: 3,
      tooltip: 'Done',
      filled: true,
      onTap: widget.onDone,
    );

    final List<Widget> rows;
    if (widget.lettersOnly) {
      rows = [
        row(_letters[1]),
        row(_letters[2]),
        row(_letters[3], trail: [backspace]),
        Row(children: [space, done]),
      ];
    } else if (_symbolPage) {
      rows = [
        row(_symbols[0]),
        row(_symbols[1]),
        row(_symbols[2]),
        row(_symbols[3], trail: [backspace]),
        Row(
          children: [
            _Key(
              key: const Key('key-letters'),
              label: 'abc',
              flex: 3,
              onTap: () => setState(() => _symbolPage = false),
            ),
            for (final c in _symbols[4].split(''))
              _Key(key: Key('key-$c'), label: c, onTap: () => _press(c)),
            space,
            done,
          ],
        ),
      ];
    } else {
      rows = [
        row(_letters[0]),
        row(_letters[1]),
        row(_letters[2]),
        row(
          _letters[3],
          lead: [
            _Key(
              key: const Key('key-shift'),
              icon: _shift == _Shift.locked
                  ? Icons.keyboard_capslock
                  : Icons.arrow_upward,
              flex: 3,
              tooltip: 'Shift',
              highlighted: upper,
              onTap: _toggleShift,
            ),
          ],
          trail: [backspace],
        ),
        Row(
          children: [
            _Key(
              key: const Key('key-symbols'),
              label: '?123',
              flex: 3,
              onTap: () => setState(() => _symbolPage = true),
            ),
            space,
            done,
          ],
        ),
      ];
    }
    return Material(
      color: scheme.surfaceContainerHighest,
      borderRadius: BorderRadius.circular(12),
      child: Padding(
        padding: const EdgeInsets.all(4),
        child: Column(mainAxisSize: MainAxisSize.min, children: rows),
      ),
    );
  }
}

class _Key extends StatelessWidget {
  const _Key({
    super.key,
    this.label,
    this.icon,
    this.flex = 2,
    this.tooltip,
    this.filled = false,
    this.highlighted = false,
    required this.onTap,
    this.onLongPress,
  });

  final String? label;
  final IconData? icon;
  final int flex;
  final String? tooltip;
  final bool filled;
  final bool highlighted;
  final VoidCallback onTap;
  final VoidCallback? onLongPress;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final bg = filled
        ? scheme.primary
        : highlighted
        ? scheme.secondaryContainer
        : scheme.surface;
    final fg = filled ? scheme.onPrimary : scheme.onSurface;
    Widget child = icon != null
        ? Icon(icon, size: 20, color: fg)
        : Text(label!, style: TextStyle(fontSize: 17, color: fg));
    if (tooltip != null) child = Tooltip(message: tooltip!, child: child);
    return Expanded(
      flex: flex,
      child: Padding(
        padding: const EdgeInsets.all(2),
        child: Material(
          color: bg,
          borderRadius: BorderRadius.circular(6),
          child: InkWell(
            borderRadius: BorderRadius.circular(6),
            onTap: () {
              HapticFeedback.selectionClick();
              onTap();
            },
            onLongPress: onLongPress,
            child: SizedBox(height: 44, child: Center(child: child)),
          ),
        ),
      ),
    );
  }
}

/// Coloured bar and advice for a new password, scored by zxcvbn in Rust.
class StrengthMeter extends StatelessWidget {
  const StrengthMeter({super.key, required this.strength});

  final Strength? strength;

  @override
  Widget build(BuildContext context) {
    final s = strength;
    final scheme = Theme.of(context).colorScheme;
    if (s == null) return const SizedBox.shrink();
    const labels = ['Very weak', 'Weak', 'Fair', 'Strong', 'Very strong'];
    final color = switch (s.score) {
      0 || 1 => scheme.error,
      2 => Colors.orange,
      _ => Colors.green,
    };
    final advice = [
      if (s.warning.isNotEmpty) s.warning,
      if (s.suggestion.isNotEmpty) s.suggestion,
      if (!s.acceptable && s.warning.isEmpty && s.suggestion.isEmpty)
        'Use at least 10 characters; a few unrelated words work well.',
    ].join(' ');
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        const SizedBox(height: 8),
        ClipRRect(
          borderRadius: BorderRadius.circular(4),
          child: LinearProgressIndicator(
            value: (s.score + 1) / 5,
            color: color,
            minHeight: 6,
          ),
        ),
        const SizedBox(height: 4),
        Text(
          labels[s.score.clamp(0, 4)],
          key: const Key('strength-label'),
          style: TextStyle(color: color, fontWeight: FontWeight.w600),
        ),
        if (advice.isNotEmpty)
          Text(advice, style: Theme.of(context).textTheme.bodySmall),
      ],
    );
  }
}
