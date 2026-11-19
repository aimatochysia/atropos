import 'package:flutter/material.dart';

import '../core/crypto_service.dart';
import 'update_ui.dart';

class SettingsPage extends StatelessWidget {
  const SettingsPage({super.key, required this.crypto, this.appVersion = ''});

  final CryptoService crypto;
  final String appVersion;

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(title: const Text('Settings')),
      body: ListView(
        children: [
          const _Header('Updates'),
          const UpdateTile(),
          const _Header('About'),
          ListTile(
            leading: const Icon(Icons.info_outline),
            title: const Text('Atropos'),
            subtitle: Text(
              'App $appVersion · core ${crypto.coreVersion}\n'
              'Crypto: ${crypto.backend}',
            ),
          ),
          const ListTile(
            leading: Icon(Icons.verified_user_outlined),
            title: Text('Algorithms'),
            subtitle: Text(
              'ML-KEM-1024 (FIPS 203) + ECDH P-384 key wrapping, '
              'AES-256-GCM, HKDF-SHA-384, Argon2id for passwords.',
            ),
          ),
        ],
      ),
    );
  }
}

class _Header extends StatelessWidget {
  const _Header(this.text);
  final String text;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.fromLTRB(16, 20, 16, 4),
      child: Text(
        text,
        style: Theme.of(context).textTheme.labelLarge
            ?.copyWith(color: Theme.of(context).colorScheme.primary),
      ),
    );
  }
}
