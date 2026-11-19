import 'package:atropos/app.dart';
import 'package:atropos/core/crypto_service.dart';
import 'package:atropos/state/app_controller.dart';
import 'package:atropos/ui/selftest_page.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

class FakeCrypto implements CryptoService {
  @override
  String get coreVersion => '0.1.0';

  @override
  String get backend => 'fake';

  @override
  Future<SelfTestReport> runSelfTest() async => SelfTestReport(
    passed: true,
    steps: [
      SelfTestStep(
        name: 'Hybrid KEM',
        passed: true,
        millis: BigInt.from(3),
        detail: 'ok',
      ),
    ],
  );
}

class FakeStore implements SettingsStore {
  AppSettings settings = const AppSettings(
    autoLockMinutes: 0,
    secureKeyboard: true,
    checkUpdates: true,
  );
  final known = <KnownVault>[];

  @override
  Future<AppSettings> load() async => settings;

  @override
  Future<void> save(AppSettings s) async => settings = s;

  @override
  Future<List<KnownVault>> vaults() async => List.of(known);

  @override
  Future<void> remember(String path) async {}

  @override
  Future<void> forget(String path) async =>
      known.removeWhere((v) => v.path == path);

  @override
  Future<String> deviceId() async => '00112233445566778899aabbccddeeff';
}

void main() {
  testWidgets('self-test shows its results', (tester) async {
    await tester.pumpWidget(
      MaterialApp(home: SelfTestPage(crypto: FakeCrypto())),
    );
    await tester.tap(find.byKey(const Key('run-self-test')));
    await tester.pumpAndSettle();
    expect(find.text('All checks passed'), findsOneWidget);
    expect(find.text('Hybrid KEM'), findsOneWidget);
  });

  testWidgets('home explains vaults when there are none', (tester) async {
    final controller = AppController(store: FakeStore());
    await tester.pumpWidget(
      AtroposApp(crypto: FakeCrypto(), controller: controller),
    );
    await controller.load();
    await tester.pumpAndSettle();
    expect(find.text('Create your first vault'), findsOneWidget);
    expect(find.byKey(const Key('new-vault')), findsOneWidget);
  });

  testWidgets('auto-lock setting is saved', (tester) async {
    final store = FakeStore();
    final controller = AppController(store: store);
    await controller.load();
    await tester.pumpWidget(
      AtroposApp(crypto: FakeCrypto(), controller: controller),
    );
    await tester.tap(find.byTooltip('Settings'));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('auto-lock')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('After 5 minutes in the background'));
    await tester.pumpAndSettle();
    expect(store.settings.autoLockMinutes, 5);
  });
}
