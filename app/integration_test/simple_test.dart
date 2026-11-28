import 'dart:convert';
import 'dart:io';
import 'dart:math' as math;
import 'dart:typed_data';

import 'package:atropos/app.dart';
import 'package:atropos/core/biometrics.dart';
import 'package:atropos/core/crypto_service.dart';
import 'package:atropos/core/platform_files.dart';
import 'package:atropos/src/rust/api/card.dart' as card;
import 'package:atropos/src/rust/api/maintenance.dart';
import 'package:atropos/src/rust/api/package.dart';
import 'package:atropos/src/rust/api/secure_input.dart';
import 'package:atropos/src/rust/api/vault.dart';
import 'package:atropos/src/rust/api/viewer.dart';
import 'package:atropos/src/rust/frb_generated.dart';
import 'package:atropos/state/app_controller.dart';
import 'package:atropos/ui/check_page.dart';
import 'package:atropos/ui/package_browser_page.dart';
import 'package:atropos/ui/selftest_page.dart';
import 'package:atropos/ui/update_ui.dart';
import 'package:atropos/ui/vault_page.dart';
import 'package:atropos/ui/viewer_page.dart';
import 'package:atropos/state/update_controller.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';

import '../test/fake_update.dart';

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

/// A one-page PDF (200x100 points) with a red square on the left, or
/// with [content] drawn in Helvetica.
List<int> tinyPdf({String content = '1 0 0 rg 0 0 100 100 re f'}) {
  final objects = [
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] /Contents 4 0 R '
        '/Resources << /Font << /F1 5 0 R >> >> >>',
    '<< /Length ${content.length} >>\nstream\n$content\nendstream',
    '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>',
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

/// Two seconds of a quiet 440 Hz tone, 8 kHz stereo 16-bit WAV.
Uint8List tinyWav() {
  const rate = 8000, frames = rate * 2;
  final b = BytesBuilder();
  void u32(int v) =>
      b.add((ByteData(4)..setUint32(0, v, Endian.little)).buffer.asUint8List());
  void u16(int v) =>
      b.add((ByteData(2)..setUint16(0, v, Endian.little)).buffer.asUint8List());
  b.add(ascii.encode('RIFF'));
  u32(36 + frames * 4);
  b.add(ascii.encode('WAVEfmt '));
  u32(16);
  u16(1);
  u16(2);
  u32(rate);
  u32(rate * 4);
  u16(4);
  u16(16);
  b.add(ascii.encode('data'));
  u32(frames * 4);
  for (var i = 0; i < frames; i++) {
    final v = (math.sin(i * 440 * 2 * math.pi / rate) * 4000).round() & 0xffff;
    u16(v);
    u16(v);
  }
  return b.toBytes();
}

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
      lfs: true,
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
      lfs: true,
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
      path: '${tmp.path}/vaults/Keys.atropos',
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
      lfs: true,
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
            home: ViewerPage.vault(
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
      lfs: true,
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

  testWidgets('re-key keeps the files and changes the keys', (tester) async {
    final src = Directory('${tmp.path}/src')..createSync();
    File('${src.path}/a.txt').writeAsStringSync('alpha');
    final created = await createVault(
      parentDir: tmp.path,
      name: 'Rekey',
      password: SecretInput.fromText(text: 'violet harbor tundra 9'),
      lfs: true,
    );
    final s = created.session;
    await finish(
      s.import_(sources: ['${src.path}/a.txt'], deleteOriginals: false),
    );
    final oldId = s.vaultId;
    RekeyDone? done;
    await for (final e in s.rekey(
      password: SecretInput.fromText(text: 'violet harbor tundra 9'),
    )) {
      done = e.done() ?? done;
    }
    expect(done, isNotNull);
    expect(s.isLocked(), isTrue);
    final fresh = done!.session;
    expect(fresh.vaultId, isNot(oldId));
    expect(done.recoveryPhrase, isNot(created.recoveryPhrase));
    expect((await fresh.list()).map((e) => e.name), ['a.txt']);
    await fresh.lock();
    await unlockWithRecoveryPhrase(
      path: fresh.path,
      phrase: SecretInput.fromText(text: done.recoveryPhrase),
    );
  });
  testWidgets('a package opens in the app without extracting', (tester) async {
    tester.view.physicalSize = const Size(900, 1800);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    final src = Directory('${tmp.path}/trip')..createSync();
    Directory('${src.path}/docs').createSync();
    File('${src.path}/docs/plan.txt').writeAsStringSync('meet at noon');
    File('${src.path}/song.wav').writeAsBytesSync(tinyWav());
    final out = '${tmp.path}/trip.atropos';
    await finish(
      sealFiles(
        sources: [src.path],
        outFile: out,
        passphrase: SecretInput.fromText(text: 'violet harbor tundra 9'),
      ),
    );
    await expectLater(
      browsePackage(
        path: out,
        passphrase: SecretInput.fromText(text: 'wrong passphrase 1'),
      ),
      throwsA(anything),
    );
    final p = await browsePackage(
      path: out,
      passphrase: SecretInput.fromText(text: 'violet harbor tundra 9'),
    );
    final top = await p.list();
    expect(top.map((e) => e.name), ['trip']);
    final trip = await p.list(parent: top.single.id);
    expect(trip.map((e) => e.name), ['docs', 'song.wav']);

    // Audio is read in memory; playing is not tried (CI has no speakers).
    final song = await p.openViewer(id: trip[1].id);
    expect(song.kind, PreviewKind.audio);
    expect((await song.audioInfo()).durationMs, 2000);
    expect(song.audioState().playing, isFalse);

    final controller = AppController(
      store: RustSettingsStore('${tmp.path}/config'),
      files: TempFiles(tmp.path),
    );
    await controller.load();
    controller.packageOpened(p);
    await tester.pumpWidget(
      AppScope(
        controller: controller,
        child: MaterialApp(home: PackageBrowserPage(package: p)),
      ),
    );
    await pumpUntil(tester, find.byKey(const Key('package-item-trip')));
    await tester.tap(find.byKey(const Key('package-item-trip')));
    await pumpUntil(tester, find.byKey(const Key('package-item-docs')));
    await tester.tap(find.byKey(const Key('package-item-docs')));
    await pumpUntil(tester, find.byKey(const Key('package-item-plan.txt')));
    await tester.tap(find.byKey(const Key('package-item-plan.txt')));
    await pumpUntil(tester, find.byKey(const Key('viewer-text')));
    expect(find.text('meet at noon'), findsOneWidget);

    // Text in a package can be edited; saving rewrites the package.
    await tester.tap(find.byKey(const Key('viewer-edit')));
    await pumpUntil(tester, find.byKey(const Key('editor-field')));
    await tester.enterText(
      find.byKey(const Key('editor-field')),
      'meet at one',
    );
    await tester.pump();
    await tester.tap(find.byKey(const Key('editor-save')));
    await pumpUntil(tester, find.text('Saved into the package.'));
    await pumpUntil(tester, find.byKey(const Key('viewer-text')));
    expect(find.text('meet at one'), findsOneWidget);
    final docs = (await p.list(parent: top.single.id)).first;
    final plan = (await p.list(parent: docs.id)).single;
    final reread = await p.openViewer(id: plan.id);
    expect(await reread.editableText(), 'meet at one');
    reread.close();
    // The other files are still there and readable.
    final again = await p.openViewer(id: trip[1].id);
    expect((await again.audioInfo()).durationMs, 2000);
    again.close();

    // One file can be extracted on its own.
    final dest = Directory('${tmp.path}/out')..createSync();
    final written = await p.extractFile(id: plan.id, destDir: dest.path);
    expect(File(written).readAsStringSync(), 'meet at one');
    final second = await p.extractFile(id: plan.id, destDir: dest.path);
    expect(second, isNot(written));

    // Locking closes the package and everything opened from it.
    await controller.lockAll();
    expect(p.isClosed(), isTrue);
    await expectLater(song.audioInfo(), throwsA(anything));
    await tester.pumpWidget(const SizedBox());
  });

  testWidgets('text can be edited and saved back into the vault', (
    tester,
  ) async {
    final src = Directory('${tmp.path}/src')..createSync();
    File('${src.path}/todo.txt').writeAsStringSync('buy milk\r\n');
    final created = await createVault(
      parentDir: tmp.path,
      name: 'Edit',
      password: SecretInput.fromText(text: 'violet harbor tundra 9'),
      lfs: true,
    );
    final s = created.session;
    await finish(
      s.import_(sources: ['${src.path}/todo.txt'], deleteOriginals: false),
    );
    final e = (await s.list()).single;
    final v = await s.openViewer(id: e.id);
    expect((await v.text()).editable, isTrue);
    expect(await v.editableText(), 'buy milk\r\n');
    v.close();
    await s.saveText(id: e.id, text: 'buy milk\r\nand bread\r\n');
    final again = await s.openViewer(id: e.id);
    expect(await again.editableText(), 'buy milk\r\nand bread\r\n');
    expect((await s.list()).single.size.toInt(), 21);
    again.close();
    await s.lock();
  });

  testWidgets('sorting, search, open as, next and previous, find', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(900, 1800);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    final src = Directory('${tmp.path}/src')..createSync();
    File('${src.path}/b.txt').writeAsStringSync('beta beta Beta');
    File('${src.path}/a.md').writeAsStringSync('# Alpha\n\nalpha text');
    File('${src.path}/c.cfg').writeAsStringSync('key = value\n' * 50);
    Directory('${src.path}/deep/er').createSync(recursive: true);
    File('${src.path}/deep/er/needle.txt').writeAsStringSync('x');
    final now = DateTime.now();
    File('${src.path}/b.txt').setLastModifiedSync(now);
    File('${src.path}/a.md')
        .setLastModifiedSync(now.subtract(const Duration(days: 3)));
    File('${src.path}/c.cfg')
        .setLastModifiedSync(now.subtract(const Duration(days: 9)));
    final created = await createVault(
      parentDir: tmp.path,
      name: 'Sorts',
      password: SecretInput.fromText(text: 'violet harbor tundra 9'),
      lfs: true,
    );
    final s = created.session;
    await finish(
      s.import_(
        sources: [
          for (final n in ['b.txt', 'a.md', 'c.cfg', 'deep']) '${src.path}/$n',
        ],
        deleteOriginals: false,
      ),
    );
    Future<List<String>> names(SortBy by, {bool desc = false}) async => [
      for (final e in await s.list(
        sort: SortOrder(by: by, descending: desc),
      ))
        e.name,
    ];
    // Folders first, then the order asked for.
    expect(await names(SortBy.name), ['deep', 'a.md', 'b.txt', 'c.cfg']);
    expect(await names(SortBy.name, desc: true), [
      'deep',
      'c.cfg',
      'b.txt',
      'a.md',
    ]);
    expect(await names(SortBy.modified, desc: true), [
      'deep',
      'b.txt',
      'a.md',
      'c.cfg',
    ]);
    expect(await names(SortBy.size, desc: true), [
      'deep',
      'c.cfg',
      'a.md',
      'b.txt',
    ]);
    expect(await names(SortBy.type), ['deep', 'c.cfg', 'a.md', 'b.txt']);

    final hits = await s.search(query: 'NEEDLE');
    expect(hits.single.entry.name, 'needle.txt');
    expect(hits.single.folders.map((f) => f.name), ['deep', 'er']);
    expect(await s.search(query: 'zzz'), isEmpty);

    // "Open as" is kept with the file.
    final cfg = (await s.list()).firstWhere((e) => e.name == 'c.cfg');
    await s.setOpenAs(id: cfg.id, openAs: PreviewKind.markdown);
    final cfg2 = (await s.list()).firstWhere((e) => e.name == 'c.cfg');
    expect(cfg2.openAs, PreviewKind.markdown);
    final cv = await s.openViewer(id: cfg.id);
    expect(cv.kind, PreviewKind.markdown);
    cv.close();

    final controller = AppController(
      store: RustSettingsStore('${tmp.path}/config'),
      files: TempFiles(tmp.path),
    );
    await controller.load();
    await controller.opened(s);
    await controller.setSort(
      s.path,
      const SortOrder(by: SortBy.size, descending: true),
    );
    final fresh = AppController(
      store: RustSettingsStore('${tmp.path}/config'),
      files: TempFiles(tmp.path),
    );
    await fresh.load();
    expect(fresh.sortFor(s.path).by, SortBy.size);

    final files = (await s.list()).where((e) => e.kind == EntryKind.file);
    await tester.pumpWidget(
      AppScope(
        controller: controller,
        child: MaterialApp(
          home: ViewerPage(
            entries: files.toList(),
            index: 1,
            source: vaultSource(s, export: (_) async {}),
          ),
        ),
      ),
    );
    await pumpUntil(tester, find.byKey(const Key('viewer-text')));
    expect(find.text('b.txt'), findsOneWidget);
    expect(find.text('2 of 3'), findsOneWidget);

    // Find with and without match case.
    await tester.tap(find.byKey(const Key('viewer-find')));
    await tester.pump();
    await tester.enterText(find.byKey(const Key('find-field')), 'beta');
    await pumpUntil(tester, find.text('1 of 3'));
    await tester.tap(find.byKey(const Key('find-next')));
    await pumpUntil(tester, find.text('2 of 3'));
    await tester.tap(find.byKey(const Key('find-case')));
    await pumpUntil(tester, find.text('1 of 2'));

    // Arrow keys and buttons move through the folder.
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pump();
    await tester.tap(find.byKey(const Key('viewer-next')));
    await pumpUntil(tester, find.text('c.cfg'));
    // Opened as Markdown, as chosen above.
    await pumpUntil(tester, find.byKey(const Key('viewer-markdown')));
    await tester.tap(find.byKey(const Key('viewer-next')));
    await pumpUntil(tester, find.text('a.md'));
    await pumpUntil(tester, find.byKey(const Key('viewer-markdown')));
    await tester.tap(find.byKey(const Key('viewer-previous')));
    await pumpUntil(tester, find.text('c.cfg'));
    await tester.pumpWidget(const SizedBox());
    await s.lock();
  });

  testWidgets('pictures are edited and saved, or saved as a copy', (
    tester,
  ) async {
    final src = Directory('${tmp.path}/src')..createSync();
    File('${src.path}/dot.png').writeAsBytesSync(base64.decode(tinyPng));
    final created = await createVault(
      parentDir: tmp.path,
      name: 'Pics',
      password: SecretInput.fromText(text: 'violet harbor tundra 9'),
      lfs: true,
    );
    final s = created.session;
    await finish(
      s.import_(sources: ['${src.path}/dot.png'], deleteOriginals: false),
    );
    final e = (await s.list()).single;
    final v = await s.openViewer(id: e.id);
    expect(await v.photoDetails(), isEmpty);
    final preview = await v.renderEdited(
      edits: const [ImageEdit.resize(maxSide: 1), ImageEdit.rotateLeft()],
      maxWidth: 100,
      maxHeight: 100,
    );
    expect(preview.width, greaterThan(0));
    await expectLater(
      v.renderEdited(
        edits: const [ImageEdit.crop(left: 0.8, top: 0, right: 0.2, bottom: 1)],
        maxWidth: 10,
        maxHeight: 10,
      ),
      throwsA(anything),
    );
    final copyId = await s.saveEditedImage(
      viewer: v,
      id: e.id,
      edits: const [ImageEdit.flipHorizontal()],
      asCopy: true,
    );
    expect(copyId, isNot(e.id));
    expect((await s.list()).map((x) => x.name).toSet(), {
      'dot.png',
      'dot (edited).png',
    });
    final same = await s.saveEditedImage(
      viewer: v,
      id: e.id,
      edits: const [ImageEdit.rotateRight()],
      asCopy: false,
    );
    expect(same, e.id);
    v.close();
    final again = await s.openViewer(id: e.id);
    expect(again.kind, PreviewKind.image);
    again.close();
    await s.lock();
  });

  testWidgets('find in a PDF lists the pages', (tester) async {
    final src = Directory('${tmp.path}/src')..createSync();
    File('${src.path}/memo.pdf').writeAsBytesSync(
      tinyPdf(
        content:
            'BT /F1 12 Tf 10 70 Td (Quarterly plan) Tj 0 -20 Td '
            '(Atropos keeps the plan safe) Tj ET',
      ),
    );
    final created = await createVault(
      parentDir: tmp.path,
      name: 'Pdf',
      password: SecretInput.fromText(text: 'violet harbor tundra 9'),
      lfs: true,
    );
    final s = created.session;
    await finish(
      s.import_(sources: ['${src.path}/memo.pdf'], deleteOriginals: false),
    );
    final v = await s.openViewer(id: (await s.list()).single.id);
    final hits = await v.pdfSearch(query: 'PLAN', matchCase: false);
    expect(hits.single.page, 0);
    expect(hits.single.count, 2);
    expect(await v.pdfSearch(query: 'PLAN', matchCase: true), isEmpty);
    v.close();
    await s.lock();
  });

  testWidgets('recent packages are listed by name and can be forgotten', (
    tester,
  ) async {
    final src = Directory('${tmp.path}/box')..createSync();
    File('${src.path}/a.txt').writeAsStringSync('a');
    final out = '${tmp.path}/box.atropos';
    await finish(
      sealFiles(
        sources: [src.path],
        outFile: out,
        passphrase: SecretInput.fromText(text: 'violet harbor tundra 9'),
      ),
    );
    final c = AppController(
      store: RustSettingsStore('${tmp.path}/config'),
      files: TempFiles(tmp.path),
    );
    await c.load();
    await c.packageUsed(out);
    expect(c.packages.single.name, 'box');
    expect(c.packages.single.path, out);
    await c.setRememberPackages(false);
    expect(c.packages, isEmpty);
    await c.packageUsed(out);
    expect(c.packages, isEmpty);
    await c.setRememberPackages(true);
    await c.packageUsed(out);
    await c.forgetPackage(out);
    expect(c.packages, isEmpty);
  });

  testWidgets('a single-file vault: notes, folders, cut, copy and drag', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(900, 1800);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    const pw = 'violet harbor tundra 9';
    final created = await createVault(
      parentDir: tmp.path,
      name: 'One',
      password: SecretInput.fromText(text: pw),
      lfs: false,
    );
    final s = created.session;
    final file = '${tmp.path}/One.atropos';
    expect(s.isSingle, isTrue);
    expect(s.path, file);
    expect(File(file).existsSync(), isTrue);
    final controller = AppController(
      store: RustSettingsStore('${tmp.path}/config'),
      files: TempFiles(tmp.path),
    );
    await controller.load();
    await controller.opened(s);
    await tester.pumpWidget(
      AppScope(
        controller: controller,
        child: MaterialApp(home: VaultPage(session: s)),
      ),
    );
    await pumpUntil(tester, find.byKey(const Key('vault-add')));

    Future<void> addMenu(String key) async {
      await tester.tap(find.byKey(const Key('vault-add')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(Key(key)));
      await tester.pumpAndSettle();
    }

    Future<void> prompt(String text) async {
      await tester.enterText(find.byKey(const Key('prompt-field')), text);
      await tester.tap(find.byKey(const Key('prompt-ok')));
      await tester.pumpAndSettle();
    }

    // A new Markdown note opens straight in the editor.
    await addMenu('new-note');
    await prompt('Note.md');
    await pumpUntil(tester, find.byKey(const Key('editor-field')));
    await tester.enterText(find.byKey(const Key('editor-field')), '# Hello');
    await tester.pump();
    await tester.tap(find.byKey(const Key('editor-save')));
    await pumpUntil(tester, find.byKey(const Key('entry-Note.md')));

    // A text file, and a folder.
    await addMenu('new-text');
    await prompt('b.txt');
    await pumpUntil(tester, find.byKey(const Key('editor-field')));
    await tester.enterText(find.byKey(const Key('editor-field')), 'bee');
    await tester.pump();
    await tester.tap(find.byKey(const Key('editor-save')));
    await pumpUntil(tester, find.byKey(const Key('entry-b.txt')));
    await addMenu('new-folder');
    await prompt('Docs');
    await pumpUntil(tester, find.byKey(const Key('entry-Docs')));

    // Cut the note, open the folder, Move here.
    await tester.tap(find.byKey(const Key('more-Note.md')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('item-cut')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('paste-here')), findsOneWidget);
    await tester.tap(find.byKey(const Key('entry-Docs')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('paste-here')));
    await pumpUntil(tester, find.byKey(const Key('entry-Note.md')));
    expect(find.byKey(const Key('paste-here')), findsNothing);
    await tester.tap(find.byKey(const Key('crumb-0')));
    await pumpUntil(tester, find.byKey(const Key('entry-b.txt')));
    expect(find.byKey(const Key('entry-Note.md')), findsNothing);

    // Drag the text file onto the folder.
    final g = await tester.startGesture(
      tester.getCenter(find.byKey(const Key('entry-b.txt'))),
    );
    await tester.pump(const Duration(milliseconds: 50));
    final target = tester.getCenter(find.byKey(const Key('entry-Docs')));
    for (var i = 1; i <= 10; i++) {
      final from = tester.getCenter(find.byKey(const Key('entry-b.txt')));
      await g.moveTo(Offset.lerp(from, target, i / 10)!);
      await tester.pump(const Duration(milliseconds: 20));
    }
    await g.up();
    await tester.pumpAndSettle();
    await pumpUntil(tester, find.textContaining('Moved "b.txt" to Docs'));
    final top = await s.list();
    expect(top.map((e) => e.name), ['Docs']);
    final docs = top.single;
    final inDocs = await s.list(parent: docs.id);
    expect(inDocs.map((e) => e.name), ['b.txt', 'Note.md']);

    // Make a copy of the folder, and see its size in Details.
    await tester.tap(find.byKey(const Key('more-Docs')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('item-duplicate')));
    await pumpUntil(tester, find.byKey(const Key('entry-Docs (2)')));
    await tester.tap(find.byKey(const Key('more-Docs (2)')));
    await tester.pumpAndSettle();
    await tester.ensureVisible(find.byKey(const Key('item-details')));
    await tester.tap(find.byKey(const Key('item-details')));
    await pumpUntil(tester, find.byKey(const Key('details-rows')));
    expect(find.text('2 files in 0 folders'), findsOneWidget);
    await tester.tap(find.text('Close'));
    await tester.pumpAndSettle();

    // Everything is in the file: lock, unlock and read it back.
    await controller.lock(s.path);
    await tester.pumpWidget(const SizedBox());
    final again = await unlockWithPassword(
      path: file,
      password: SecretInput.fromText(text: pw),
    );
    final copy = (await again.list()).firstWhere((e) => e.name == 'Docs (2)');
    final note = (await again.list(parent: copy.id))
        .firstWhere((e) => e.name == 'Note.md');
    final v = await again.openViewer(id: note.id);
    expect(await v.editableText(), '# Hello');
    v.close();

    // A Vault LFS copy opens with the same password.
    await finish(again.convert(out: '${tmp.path}/One LFS'));
    final lfs = await unlockWithPassword(
      path: '${tmp.path}/One LFS',
      password: SecretInput.fromText(text: pw),
    );
    expect(lfs.isSingle, isFalse);
    expect((await lfs.list()).map((e) => e.name), ['Docs', 'Docs (2)']);
    await lfs.lock();
    await again.lock();
  });

  testWidgets('an older passphrase file is upgraded from Home', (tester) async {
    tester.view.physicalSize = const Size(900, 1800);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    const pass = 'violet harbor tundra';
    final src = Directory('${tmp.path}/box')..createSync();
    File('${src.path}/a.txt').writeAsStringSync('apple');
    final out = '${tmp.path}/box.atropos';
    await finish(
      sealFiles(
        sources: [src.path],
        outFile: out,
        passphrase: SecretInput.fromText(text: pass),
      ),
    );
    expect(isOldPackage(path: out), isTrue);
    expect(isVault(path: out), isFalse);
    final controller = AppController(
      store: RustSettingsStore('${tmp.path}/config'),
      files: TempFiles(tmp.path),
    );
    await controller.load();
    await controller.packageUsed(out);
    await tester.pumpWidget(
      AtroposApp(crypto: const RustCryptoService(), controller: controller),
    );
    await pumpUntil(tester, find.byKey(const Key('recent-box')));
    await tester.tap(find.byKey(const Key('recent-box')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('upgrade-go')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('secret-first')));
    await tester.pumpAndSettle();
    await typeOnKeypad(tester, pass);
    await tester.tap(find.byKey(const Key('secret-ok')));
    await pumpUntil(tester, find.byKey(const Key('recovery-written')));
    await tester.ensureVisible(find.byKey(const Key('recovery-written')));
    await tester.tap(find.byKey(const Key('recovery-written')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('recovery-continue')));
    await pumpUntil(tester, find.byKey(const Key('entry-box')));
    expect(isVault(path: out), isTrue);
    expect(controller.packages, isEmpty);
    expect(controller.vaults.map((v) => v.name), contains('box'));
    final s = controller.session(out)!;
    final box = (await s.list()).single;
    final a = (await s.list(parent: box.id)).single;
    final v = await s.openViewer(id: a.id);
    expect(await v.editableText(), 'apple');
    v.close();
    await controller.lockAll();
    await tester.pumpWidget(const SizedBox());
  });

  testWidgets('a new release pops up once and can be skipped', (tester) async {
    final c = AppController(
      store: RustSettingsStore('${tmp.path}/config'),
      files: TempFiles(tmp.path),
    );
    await c.load();
    expect(c.settings.updateCheck, UpdateCheck.everyStart);
    final svc = FakeUpdateService()..next = updateInfo(version: '9.0.0');
    final updates = UpdateController(service: svc, currentVersion: '0.7.0');
    await tester.pumpWidget(
      AtroposApp(
        crypto: const RustCryptoService(),
        controller: c,
        updates: updates,
      ),
    );
    await tester.pump();
    final check = startupUpdateCheck(
      c,
      updates,
      () => appNavigator.currentContext,
      delay: Duration.zero,
    );
    await pumpUntil(tester, find.byKey(const Key('update-dialog')));
    expect(find.textContaining('shiny', findRichText: true), findsOneWidget);
    await tester.tap(find.byKey(const Key('update-skip')));
    await tester.pumpAndSettle();
    await check;
    expect(c.settings.skippedVersion, '9.0.0');
    expect(c.settings.notesShownFor, '9.0.0');
    expect(c.settings.lastUpdateCheck, greaterThan(0));
    expect(find.byKey(const Key('update-banner')), findsNothing);

    // Next start: skipped, so no popup.
    await startupUpdateCheck(
      c,
      updates,
      () => appNavigator.currentContext,
      delay: Duration.zero,
    );
    await tester.pump();
    expect(find.byKey(const Key('update-dialog')), findsNothing);

    // Off means no check at all.
    await c.updateSettings(c.settings.copyWith(updateCheck: UpdateCheck.off));
    svc.next = updateInfo(version: '9.1.0');
    await startupUpdateCheck(
      c,
      updates,
      () => appNavigator.currentContext,
      delay: Duration.zero,
    );
    expect(updates.available?.version, '9.0.0');
    await tester.pumpWidget(const SizedBox());
  });
}
