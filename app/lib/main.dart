import 'package:flutter/material.dart';
import 'package:package_info_plus/package_info_plus.dart';

import 'app.dart';
import 'core/crypto_service.dart';
import 'core/update_service.dart';
import 'src/rust/frb_generated.dart';
import 'state/update_controller.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  await RustLib.init();
  final info = await PackageInfo.fromPlatform();
  final updates = UpdateController(
    service: RustUpdateService(),
    currentVersion: info.version,
  );
  runApp(
    AtroposApp(
      crypto: const RustCryptoService(),
      updates: updates,
      appVersion: info.version,
    ),
  );
  // Give startup a moment before going to the network.
  await Future<void>.delayed(const Duration(seconds: 3));
  await updates.check(silent: true);
}
