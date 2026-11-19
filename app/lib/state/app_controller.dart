import 'dart:async';

import 'package:flutter/widgets.dart';

import '../core/platform_files.dart';
import '../src/rust/api/settings.dart' as rs;
import '../src/rust/api/vault.dart';

export '../src/rust/api/settings.dart' show AppSettings, KnownVault;

/// Where settings and the list of known vaults are kept. The real one is the
/// Rust core; tests use a fake.
abstract interface class SettingsStore {
  Future<rs.AppSettings> load();
  Future<void> save(rs.AppSettings settings);
  Future<List<rs.KnownVault>> vaults();
  Future<void> remember(String path);
  Future<void> forget(String path);
}

class RustSettingsStore implements SettingsStore {
  RustSettingsStore(this.configDir);
  final String configDir;

  @override
  Future<rs.AppSettings> load() => rs.loadSettings(configDir: configDir);

  @override
  Future<void> save(rs.AppSettings settings) =>
      rs.saveSettings(configDir: configDir, settings: settings);

  @override
  Future<List<rs.KnownVault>> vaults() => rs.listVaults(configDir: configDir);

  @override
  Future<void> remember(String path) =>
      rs.rememberVault(configDir: configDir, path: path);

  @override
  Future<void> forget(String path) =>
      rs.forgetVault(configDir: configDir, path: path);
}

/// Auto-lock choices offered in settings, in minutes (0 = when the app
/// leaves the screen, including screen off).
const autoLockChoices = [0, 2, 5, 10, 30, 60];

String autoLockLabel(int minutes) => switch (minutes) {
  0 => 'When the screen turns off or I leave the app',
  60 => 'After 1 hour in the background',
  _ => 'After $minutes minutes in the background',
};

/// App-wide state: settings, known vaults, unlocked vaults and auto-lock.
class AppController extends ChangeNotifier with WidgetsBindingObserver {
  AppController({required this.store, this.files = const PlatformFiles()});

  final SettingsStore store;
  final PlatformFiles files;

  rs.AppSettings settings = const rs.AppSettings(
    autoLockMinutes: 0,
    secureKeyboard: true,
    checkUpdates: true,
  );
  List<rs.KnownVault> vaults = const [];
  bool loaded = false;

  final Map<String, VaultSession> _open = {};
  int _picking = 0;
  DateTime? _leftAt;
  Timer? _lockTimer;

  Future<void> load() async {
    try {
      settings = await store.load();
    } catch (_) {
      // Keep the defaults; a broken settings file must not block vaults.
    }
    await refreshVaults();
    loaded = true;
    notifyListeners();
  }

  Future<void> refreshVaults() async {
    try {
      vaults = await store.vaults();
    } catch (_) {
      vaults = const [];
    }
    notifyListeners();
  }

  Future<void> updateSettings(rs.AppSettings next) async {
    settings = next;
    notifyListeners();
    await store.save(next);
  }

  VaultSession? session(String path) => _open[path];
  bool isOpen(String path) => _open.containsKey(path);
  bool get anyOpen => _open.isNotEmpty;

  /// Registers a freshly unlocked or created vault.
  Future<void> opened(VaultSession s) async {
    _open[s.path] = s;
    notifyListeners();
    try {
      await store.remember(s.path);
    } catch (_) {
      // The vault still works; it just won't be listed next time.
    }
    await refreshVaults();
  }

  Future<void> forget(String path) async {
    await lock(path);
    await store.forget(path);
    await refreshVaults();
  }

  Future<void> lock(String path) async {
    final s = _open.remove(path);
    notifyListeners();
    if (s != null) await s.lock();
  }

  Future<void> lockAll() async {
    final sessions = _open.values.toList();
    _open.clear();
    _lockTimer?.cancel();
    notifyListeners();
    for (final s in sessions) {
      await s.lock();
    }
  }

  /// Runs a system picker without auto-locking: on Android the picker puts
  /// Atropos in the background.
  Future<T> whilePicking<T>(Future<T> Function() pick) async {
    _picking++;
    try {
      return await pick();
    } finally {
      // The resume event can arrive just after the picker returns.
      Future<void>.delayed(const Duration(seconds: 1), () => _picking--);
    }
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    switch (state) {
      case AppLifecycleState.paused:
      case AppLifecycleState.hidden:
        _wentAway();
      case AppLifecycleState.detached:
        lockAll();
      case AppLifecycleState.resumed:
        _cameBack();
      case AppLifecycleState.inactive:
        break;
    }
  }

  void _wentAway() {
    if (_picking > 0 || !anyOpen || _leftAt != null) return;
    final minutes = settings.autoLockMinutes;
    if (minutes <= 0) {
      lockAll();
      return;
    }
    _leftAt = DateTime.now();
    _lockTimer?.cancel();
    _lockTimer = Timer(Duration(minutes: minutes), lockAll);
  }

  void _cameBack() {
    _lockTimer?.cancel();
    final left = _leftAt;
    _leftAt = null;
    if (left == null || _picking > 0) return;
    // Timers may not run while Android has the app suspended.
    if (DateTime.now().difference(left).inMinutes >= settings.autoLockMinutes) {
      lockAll();
    }
  }

  @override
  void dispose() {
    _lockTimer?.cancel();
    super.dispose();
  }
}

/// Makes the [AppController] reachable from every page.
class AppScope extends InheritedNotifier<AppController> {
  const AppScope({
    super.key,
    required AppController controller,
    required super.child,
  }) : super(notifier: controller);

  static AppController of(BuildContext context) =>
      context.dependOnInheritedWidgetOfExactType<AppScope>()!.notifier!;

  static AppController read(BuildContext context) =>
      context.getInheritedWidgetOfExactType<AppScope>()!.notifier!;
}
