import 'package:flutter/material.dart';

import 'core/crypto_service.dart';
import 'state/app_controller.dart';
import 'state/update_controller.dart';
import 'state/update_scope.dart';
import 'ui/home_page.dart';
import 'ui/theme.dart';

/// Lets startup work (the update popup) show dialogs.
final appNavigator = GlobalKey<NavigatorState>();

class AtroposApp extends StatefulWidget {
  const AtroposApp({
    super.key,
    required this.crypto,
    required this.controller,
    this.updates,
    this.appVersion = '',
  });

  final CryptoService crypto;
  final AppController controller;
  final UpdateController? updates;
  final String appVersion;

  @override
  State<AtroposApp> createState() => _AtroposAppState();
}

class _AtroposAppState extends State<AtroposApp> {
  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(widget.controller);
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(widget.controller);
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final u = widget.updates;
    final Widget root = AppScope(
      controller: widget.controller,
      child: MaterialApp(
        navigatorKey: appNavigator,
        title: 'Atropos',
        debugShowCheckedModeBanner: false,
        theme: AtroposTheme.light,
        darkTheme: AtroposTheme.dark,
        home: HomePage(crypto: widget.crypto, appVersion: widget.appVersion),
      ),
    );
    return u == null ? root : UpdateScope(updates: u, child: root);
  }
}
