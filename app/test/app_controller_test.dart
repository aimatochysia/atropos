import 'package:atropos/state/app_controller.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';

import 'home_test.dart' show FakeStore;

void main() {
  test('labels every auto-lock choice', () {
    for (final c in autoLockChoices) {
      expect(autoLockLabel(c), isNotEmpty);
    }
    expect(autoLockLabel(0), contains('screen turns off'));
  });

  test('going to the background with nothing open does nothing', () async {
    final c = AppController(store: FakeStore());
    await c.load();
    c.didChangeAppLifecycleState(AppLifecycleState.paused);
    c.didChangeAppLifecycleState(AppLifecycleState.resumed);
    expect(c.anyOpen, isFalse);
  });
}
