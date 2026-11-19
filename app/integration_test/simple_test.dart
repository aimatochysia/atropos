import 'package:atropos/app.dart';
import 'package:atropos/core/crypto_service.dart';
import 'package:atropos/src/rust/frb_generated.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  setUpAll(() async => RustLib.init());

  testWidgets('the real Rust crypto self-test passes', (tester) async {
    await tester.pumpWidget(const AtroposApp(crypto: RustCryptoService()));
    await tester.tap(find.byKey(const Key('run-self-test')));
    for (var i = 0; i < 120; i++) {
      await tester.pump(const Duration(milliseconds: 500));
      if (find.byKey(const Key('self-test-result')).evaluate().isNotEmpty) {
        break;
      }
    }
    expect(find.text('All checks passed'), findsOneWidget);
  });
}
