import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:atropos/app.dart';
import 'package:atropos/core/biometrics.dart';
import 'package:atropos/core/crypto_service.dart';
import 'package:atropos/core/platform_files.dart';
import 'package:atropos/src/rust/api/card.dart' as card;
import 'package:atropos/src/rust/api/maintenance.dart';
import 'package:atropos/src/rust/api/secure_input.dart';
import 'package:atropos/src/rust/api/vault.dart';
import 'package:atropos/src/rust/api/viewer.dart';
import 'package:atropos/src/rust/frb_generated.dart';
import 'package:atropos/state/app_controller.dart';
import 'package:atropos/ui/check_page.dart';
import 'package:atropos/ui/selftest_page.dart';
import 'package:atropos/ui/viewer_page.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';

class TempFiles extends PlatformFiles {
  const TempFiles(this.root);
  final String root;

  @override
  Future<String> configDir() async => '$root/config';

  @override
  Future<String> defaultVaultParent() async => '$root/vaults';
}

/// Stands in for the Android Keystore: keeps secrets in memory and always
/// "recognises" the fingerprint.
class FakeBiometrics implements Biometrics {
  final Map<String, Uint8List> keys = {};

  @override
  Future<BiometricStatus> status() async => BiometricStatus.ready;

  @override
  Future<bool> has(String vaultId) async => keys.containsKey(vaultId);

  @override
  Future<bool> enroll(String vaultId, Uint8List secret) async {
    keys[vaultId] = Uint8List.fromList(secret);
    secret.fillRange(0, secret.length, 0);
    return true;
  }

  @override
  Future<Uint8List?> unlock(String vaultId, {String? title}) async =>
      keys[vaultId] == null ? null : Uint8List.fromList(keys[vaultId]!);

  @override
  Future<void> remove(String vaultId) async => keys.remove(vaultId);
}

Future<OutcomeInfo> finish(Stream<TaskEvent> s) async {
  OutcomeInfo? done;
  await for (final e in s) {
    if (e is TaskEvent_Done) done = e.field0;
  }
  return done!;
}

Future<void> pumpUntil(WidgetTester tester, Finder f) async {
  for (var i = 0; i < 240; i++) {
    await tester.pump(const Duration(milliseconds: 250));
    if (f.evaluate().isNotEmpty) return;
  }
  fail('timed out waiting for $f');
}

/// Types lowercase letters and spaces on the Atropos keypad.
Future<void> typeOnKeypad(WidgetTester tester, String text) async {
  for (final ch in text.split('')) {
    final key = ch == ' ' ? 'key-space' : 'key-$ch';
    await tester.tap(find.byKey(Key(key)).last);
    await tester.pump();
  }
}

/// A one-page PDF (200x100 points) with a red square on the left.
List<int> tinyPdf() {
  const content = '1 0 0 rg 0 0 100 100 re f';
  final objects = [
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] /Contents 4 0 R >>',
    '<< /Length ${content.length} >>\nstream\n$content\nendstream',
  ];
  final out = StringBuffer('%PDF-1.4\n');
  final offsets = <int>[];
  for (var i = 0; i < objects.length; i++) {
    offsets.add(out.length);
    out.write('${i + 1} 0 obj\n${objects[i]}\nendobj\n');
  }
  final xref = out.length;
  out.write('xref\n0 ${objects.length + 1}\n0000000000 65535 f \n');
  for (final o in offsets) {
    out.write('${o.toString().padLeft(10, '0')} 00000 n \n');
  }
  out.write(
    'trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\n'
    'startxref\n$xref\n%%EOF\n',
  );
  return ascii.encode(out.toString());
}

const tinyPng =
    'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFBQIAX8jx0gAAAABJRU5ErkJggg==';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  setUpAll(() async => RustLib.init());

  late Directory tmp;
  setUp(() => tmp = Directory.systemTemp.createTempSync('atropos-it-'));
  tearDown(() => tmp.deleteSync(recursive: true));

  testWidgets('the real Rust crypto self-test passes', (tester) async {
    await tester.pumpWidget(
      const MaterialApp(home: SelfTestPage(crypto: RustCryptoService())),
    );
    await tester.tap(find.byKey(const Key('run-self-test')));
    await pumpUntil(tester, find.byKey(const Key('self-test-result')));
    expect(find.text('All checks passed'), findsOneWidget);
  });

  testWidgets('vault round trip through the real core', (tester) async {
    final src = Directory('${tmp.path}/src/photos')
      ..createSync(recursive: true);
    File('${src.path}/a.txt').writeAsStringSync('hello vault');
    File('${src.path}/big.bin')
        .writeAsBytesSync(List.generate(300 * 1024, (i) => i % 251));

    final created = await createVault(
      parentDir: tmp.path,
      name: 'Test',
      password: SecretInput.fromText(text: 'violet harbor tundra 9'),
    );
    final phrase = created.recoveryPhrase;
    expect(phrase.split(' '), hasLength(24));
    var s = created.session;

    final imported = await finish(
      s.import_(sources: [src.path], deleteOriginals: false),
    );
    expect(imported.files, BigInt.two);
    final top = await s.list();
    expect(top.single.name, 'photos');
    final inner = await s.list(parent: top.single.id);
    expect(inner.map((e) => e.name), containsAll(['a.txt', 'big.bin']));

    // Nothing readable on disk.
    for (final f in Directory('${tmp.path}/Test').listSync(recursive: true)) {
      if (f is File) {
        expect(f.readAsBytesSync().length, isNot(11));
        expect(
          String.fromCharCodes(f.readAsBytesSync()).contains('hello vault'),
          isFalse,
        );
      }
    }

    await s.lock();
    expect(s.isLocked(), isTrue);

    await expectLater(
      unlockWithPassword(
        path: s.path,
        password: SecretInput.fromText(text: 'wrong password here'),
      ),
      throwsA(anything),
    );
    s = await unlockWithRecoveryPhrase(
      path: '${tmp.path}/Test',
      phrase: SecretInput.fromText(text: phrase),
    );

    final out = Directory('${tmp.path}/out')..createSync();
    await finish(s.export_(ids: [top.single.id], destDir: out.path));
    expect(File('${out.path}/photos/a.txt').readAsStringSync(), 'hello vault');

    // Passphrase package from the vault, extracted without the vault.
    final pkg = '${tmp.path}/photos.atropos';
    await finish(
      s.sealPackage(
        ids: [top.single.id],
        outFile: pkg,
        passphrase: SecretInput.fromText(text: 'share me please 42'),
      ),
    );
    expect((await inspectPackage(path: pkg)).needsPassphrase, isTrue);
    final ex = Directory('${tmp.path}/ex')..createSync();
    await finish(
      extractFiles(
        packageFile: pkg,
        passphrase: SecretInput.fromText(text: 'share me please 42'),
        destDir: ex.path,
      ),
    );
    expect(File('${ex.path}/photos/big.bin').lengthSync(), 300 * 1024);
    await s.lock();
  });

  testWidgets('card calls reach Dart for every APDU', (tester) async {
    // A "card" with neither applet: every SELECT is refused.
    final seen = <Uint8List>[];
    await expectLater(
      card.cardStatus(
        transmit: (apdu) {
          seen.add(apdu);
          return Uint8List.fromList([0x6A, 0x82]);
        },
      ),
      throwsA(
        predicate((e) => e.toString().contains('no OpenPGP or PIV applet')),
      ),
    );
    expect(seen.length, 2);
    expect(seen.first.sublist(0, 4), [0x00, 0xA4, 0x04, 0x00]);
    final dir = '${tmp.path}/c';
    final created = await createVault(
      parentDir: tmp.path,
      name: 'c',
      password: SecretInput.fromText(text: 'violet harbor tundra 9'),
    );
    expect(await card.vaultCardCount(path: dir), 0);
    expect(await created.session.cards(), isEmpty);
    await created.session.lock();
    // A reader-less desktop answers with a readable error or an empty list.
    try {
      expect(await card.cardReaders(), isA<List<String>>());
    } catch (e) {
      expect(e.toString(), isNotEmpty);
    }
  });

  testWidgets('create and unlock a vault with the keypad', (tester) async {
    tester.view.physicalSize = const Size(900, 1800);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    final bio = FakeBiometrics();
    final controller = AppController(
      store: RustSettingsStore('${tmp.path}/config'),
      files: TempFiles(tmp.path),
      biometrics: bio,
    );
    await controller.load();
    await tester.pumpWidget(
      AtroposApp(crypto: const RustCryptoService(), controller: controller),
    );
    await tester.pumpAndSettle();

    await tester.tap(find.byKey(const Key('new-vault')));
    await tester.pumpAndSettle();
    await tester.enterText(find.byKey(const Key('vault-name')), 'Keys');

    const pw = 'violet harbor tundra';
    await tester.tap(find.byKey(const Key('create-password')));
    await tester.pumpAndSettle();
    await typeOnKeypad(tester, pw);
    await tester.pump();
    expect(find.byKey(const Key('strength-label')), findsOneWidget);
    await tester.tap(find.byKey(const Key('key-done')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('create-password-again')));
    await tester.pumpAndSettle();
    await typeOnKeypad(tester, pw);
    // The keypad's return key submits the form once it is complete.
    await tester.tap(find.byKey(const Key('key-done')));
    await pumpUntil(tester, find.byKey(const Key('recovery-written')));
    expect(find.byKey(const Key('recovery-word-23')), findsOneWidget);
    await tester.ensureVisible(find.byKey(const Key('recovery-written')));
    await tester.tap(find.byKey(const Key('recovery-written')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('recovery-continue')));
    await tester.pumpAndSettle();
    expect(
      find.text(
        'This vault is empty. Tap Add to encrypt files or '
        'folders into it.',
      ),
      findsOneWidget,
    );

    // Lock, then unlock with the keypad.
    await tester.tap(find.byKey(const Key('lock-vault')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('vault-Keys')), findsOneWidget);
    await tester.tap(find.byKey(const Key('vault-Keys')));
    await tester.pumpAndSettle();
    await typeOnKeypad(tester, pw);
    await tester.tap(find.byKey(const Key('unlock')));
    await pumpUntil(tester, find.byKey(const Key('vault-add')));
    expect(controller.anyOpen, isTrue);

    // Auto-lock: leaving the app locks everything by default.
    controller.didChangeAppLifecycleState(AppLifecycleState.hidden);
    controller.didChangeAppLifecycleState(AppLifecycleState.paused);
    await tester.pumpAndSettle();
    expect(controller.anyOpen, isFalse);
    expect(find.byKey(const Key('new-vault')), findsOneWidget);

    // Fingerprint unlock: enrol through the core, then opening the vault
    // unlocks it without typing anything.
    final device = controller.deviceId!;
    final s = await unlockWithPassword(
      path: '${tmp.path}/vaults/Keys',
      password: SecretInput.fromText(text: pw),
      deviceId: device,
    );
    await bio.enroll(
      s.vaultId,
      await s.enrollDevice(deviceId: device, label: 'test'),
    );
    expect(await s.deviceExpires(deviceId: device), isNotNull);
    await s.lock();
    await tester.tap(find.byKey(const Key('vault-Keys')));
    await pumpUntil(tester, find.byKey(const Key('vault-add')));
    expect(controller.anyOpen, isTrue);
  });

  testWidgets('the viewer shows pictures, PDFs, text and Markdown', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(900, 1800);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    final src = Directory('${tmp.path}/src')..createSync();
    File('${src.path}/dot.png').writeAsBytesSync(base64.decode(tinyPng));
    File('${src.path}/doc.pdf').writeAsBytesSync(tinyPdf());
    File('${src.path}/notes.md')
        .writeAsStringSync('# Shopping\n\n- **milk**\n');
    File('${src.path}/log.txt').writeAsStringSync('first line\nsecond line');
    File('${src.path}/blob.bin').writeAsBytesSync([0, 1, 2, 0, 3]);
    final created = await createVault(
      parentDir: tmp.path,
      name: 'Viewer',
      password: SecretInput.fromText(text: 'violet harbor tundra 9'),
    );
    final s = created.session;
    await finish(
      s.import_(
        sources: [
          for (final n in [
            'dot.png',
            'doc.pdf',
            'notes.md',
            'log.txt',
            'blob.bin',
          ])
            '${src.path}/$n',
        ],
        deleteOriginals: false,
      ),
    );
    final entries = {for (final e in await s.list()) e.name: e};
    final controller = AppController(
      store: RustSettingsStore('${tmp.path}/config'),
      files: TempFiles(tmp.path),
    );
    await controller.load();
    await controller.opened(s);

    Future<void> show(String name) async {
      await tester.pumpWidget(
        AppScope(
          controller: controller,
          child: MaterialApp(
            home: ViewerPage(
              key: ValueKey(name),
              session: s,
              entry: entries[name]!,
              onExport: () async {},
            ),
          ),
        ),
      );
    }

    expect(previewForName(name: 'dot.png'), PreviewKind.image);
    await show('dot.png');
    await pumpUntil(tester, find.byKey(const Key('viewer-image')));

    await show('doc.pdf');
    await pumpUntil(tester, find.byType(RawImage));
    expect(find.byKey(const Key('viewer-pdf')), findsOneWidget);

    await show('notes.md');
    await pumpUntil(tester, find.byKey(const Key('viewer-markdown')));
    expect(find.textContaining('Shopping', findRichText: true), findsOneWidget);
    expect(find.textContaining('milk', findRichText: true), findsOneWidget);

    await show('log.txt');
    await pumpUntil(tester, find.byKey(const Key('viewer-text')));
    expect(find.text('second line'), findsOneWidget);

    await show('blob.bin');
    await pumpUntil(tester, find.byKey(const Key('viewer-message')));

    // Locking wipes what an open viewer holds.
    final v = await s.openViewer(id: entries['log.txt']!.id);
    await controller.lockAll();
    await expectLater(v.text(), throwsA(anything));
    await tester.pumpWidget(const SizedBox());
  });

  testWidgets('check finds a damaged file and repair removes it', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(900, 1800);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    final src = Directory('${tmp.path}/src')..createSync();
    File('${src.path}/a.txt').writeAsStringSync('alpha');
    File('${src.path}/big.bin')
        .writeAsBytesSync(List.generate(200 * 1024, (i) => i % 251));
    final created = await createVault(
      parentDir: tmp.path,
      name: 'Check',
      password: SecretInput.fromText(text: 'violet harbor tundra 9'),
    );
    final s = created.session;
    await finish(
      s.import_(
        sources: ['${src.path}/a.txt', '${src.path}/big.bin'],
        deleteOriginals: false,
      ),
    );
    expect(s.damagedParts(), 0);
    expect(await s.passwordStrength(), PasswordStrength.standard);
    final ms = await s.setPasswordStrength(
      password: SecretInput.fromText(text: 'violet harbor tundra 9'),
      strength: PasswordStrength.light,
    );
    expect(ms, greaterThan(0));
    expect(await s.passwordStrength(), PasswordStrength.light);

    final controller = AppController(
      store: RustSettingsStore('${tmp.path}/config'),
      files: TempFiles(tmp.path),
    );
    await controller.load();
    Future<void> showCheck() => tester.pumpWidget(
      AppScope(
        controller: controller,
        child: MaterialApp(
          home: CheckPage(key: UniqueKey(), session: s),
        ),
      ),
    );
    await showCheck();
    await pumpUntil(tester, find.byKey(const Key('check-clean')));

    // Corrupt the big file's blob (the largest file in objects/).
    final blobs =
        Directory('${tmp.path}/Check/objects')
            .listSync(recursive: true)
            .whereType<File>()
            .toList()
          ..sort((a, b) => b.lengthSync().compareTo(a.lengthSync()));
    final bytes = blobs.first.readAsBytesSync();
    bytes[bytes.length - 100] ^= 1;
    blobs.first.writeAsBytesSync(bytes);

    await showCheck();
    await pumpUntil(tester, find.byKey(const Key('check-problems')));
    expect(find.textContaining('Damaged files (1)'), findsOneWidget);
    await tester.tap(find.byKey(const Key('check-remove-broken')));
    await tester.pump();
    await tester.tap(find.byKey(const Key('check-repair')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Remove'));
    await pumpUntil(tester, find.byKey(const Key('check-clean')));
    expect(find.textContaining('Removed 1 file'), findsOneWidget);
    expect((await s.list()).map((e) => e.name), ['a.txt']);
    await tester.pumpWidget(const SizedBox());
    await s.lock();
  });
}
