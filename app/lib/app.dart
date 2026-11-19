import 'package:flutter/material.dart';

import 'core/crypto_service.dart';
import 'state/update_controller.dart';
import 'state/update_scope.dart';
import 'ui/home_page.dart';
import 'ui/theme.dart';

class AtroposApp extends StatelessWidget {
  const AtroposApp({
    super.key,
    required this.crypto,
    this.updates,
    this.appVersion = '',
  });

  final CryptoService crypto;
  final UpdateController? updates;
  final String appVersion;

  @override
  Widget build(BuildContext context) {
    final u = updates;
    final Widget root = MaterialApp(
      title: 'Atropos',
      debugShowCheckedModeBanner: false,
      theme: AtroposTheme.light,
      darkTheme: AtroposTheme.dark,
      home: HomePage(crypto: crypto, appVersion: appVersion),
    );
    return u == null ? root : UpdateScope(updates: u, child: root);
  }
}
