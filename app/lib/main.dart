import 'package:flutter/material.dart';
import 'package:package_info_plus/package_info_plus.dart';

import 'app.dart';
import 'core/crypto_service.dart';
import 'core/platform_files.dart';
import 'core/update_service.dart';
import 'src/rust/frb_generated.dart';
import 'state/app_controller.dart';
import 'state/update_controller.dart';
import 'ui/update_ui.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  await RustLib.init();
  final info = await PackageInfo.fromPlatform();
  const files = PlatformFiles();
  final controller = AppController(
    store: RustSettingsStore(await files.configDir()),
    files: files,
  );
  final updates = UpdateController(
    service: RustUpdateService(),
    currentVersion: info.version,
  );
  runApp(
    AtroposApp(
      crypto: const RustCryptoService(),
      controller: controller,
      updates: updates,
      appVersion: info.version,
    ),
  );
  await controller.load();
  await startupUpdateCheck(
    controller,
    updates,
    () => appNavigator.currentContext,
  );
}
