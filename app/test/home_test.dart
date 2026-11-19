import 'package:atropos/app.dart';
import 'package:atropos/core/crypto_service.dart';
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

void main() {
  testWidgets('self-test shows its results', (tester) async {
    await tester.pumpWidget(AtroposApp(crypto: FakeCrypto()));
    expect(find.text('Atropos'), findsOneWidget);
    await tester.tap(find.byKey(const Key('run-self-test')));
    await tester.pumpAndSettle();
    expect(find.text('All checks passed'), findsOneWidget);
    expect(find.text('Hybrid KEM'), findsOneWidget);
  });
}
