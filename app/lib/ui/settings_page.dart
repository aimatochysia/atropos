import 'package:flutter/material.dart';

import '../core/crypto_service.dart';
import '../state/app_controller.dart';
import 'selftest_page.dart';
import 'update_ui.dart';

class SettingsPage extends StatelessWidget {
  const SettingsPage({super.key, required this.crypto, this.appVersion = ''});

  final CryptoService crypto;
  final String appVersion;

  @override
  Widget build(BuildContext context) {
    final app = AppScope.of(context);
    final s = app.settings;
    return Scaffold(
      appBar: AppBar(title: const Text('Settings')),
      body: ListView(
        children: [
          const _Header('Security'),
          ListTile(
            key: const Key('auto-lock'),
            leading: const Icon(Icons.timer_outlined),
            title: const Text('Lock vaults'),
            subtitle: Text(autoLockLabel(s.autoLockMinutes)),
            onTap: () async {
              final m = await showDialog<int>(
                context: context,
                builder: (ctx) => SimpleDialog(
                  title: const Text('Lock vaults'),
                  children: [
                    for (final c in autoLockChoices)
                      RadioListTile<int>(
                        value: c,
                        // ignore: deprecated_member_use
                        groupValue: s.autoLockMinutes,
                        title: Text(autoLockLabel(c)),
                        // ignore: deprecated_member_use
                        onChanged: (v) => Navigator.pop(ctx, v),
                      ),
                  ],
                ),
              );
              if (m != null) {
                await app.updateSettings(
                  AppSettings(
                    autoLockMinutes: m,
                    secureKeyboard: s.secureKeyboard,
                    checkUpdates: s.checkUpdates,
                  ),
                );
              }
            },
          ),
          SwitchListTile(
            key: const Key('secure-keyboard'),
            secondary: const Icon(Icons.keyboard_outlined),
            title: const Text('Atropos keypad for passwords'),
            subtitle: const Text(
              'Passwords go straight into protected memory and are wiped '
              'after use. The system keyboard, its suggestions and keyboard '
              'apps never see them.',
            ),
            value: s.secureKeyboard,
            onChanged: (v) => app.updateSettings(
              AppSettings(
                autoLockMinutes: s.autoLockMinutes,
                secureKeyboard: v,
                checkUpdates: s.checkUpdates,
              ),
            ),
          ),
          ListTile(
            leading: const Icon(Icons.fact_check_outlined),
            title: const Text('Cryptography self-test'),
            subtitle: const Text('Check every algorithm on this device.'),
            onTap: () => Navigator.of(context).push(
              MaterialPageRoute<void>(
                builder: (_) => SelfTestPage(crypto: crypto),
              ),
            ),
          ),
          const _Header('Updates'),
          const UpdateTile(),
          SwitchListTile(
            secondary: const Icon(Icons.update),
            title: const Text('Check for updates when Atropos starts'),
            subtitle: const Text(
              'Asks GitHub for the latest release. Nothing about your vaults '
              'is sent.',
            ),
            value: s.checkUpdates,
            onChanged: (v) => app.updateSettings(
              AppSettings(
                autoLockMinutes: s.autoLockMinutes,
                secureKeyboard: s.secureKeyboard,
                checkUpdates: v,
              ),
            ),
          ),
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
