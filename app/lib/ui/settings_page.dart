import 'package:flutter/material.dart';

import '../core/crypto_service.dart';
import '../state/app_controller.dart';
import 'selftest_page.dart';
import 'update_ui.dart';
import 'widgets/feedback.dart';

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
                await app.updateSettings(s.copyWith(autoLockMinutes: m));
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
            onChanged: (v) => app.updateSettings(s.copyWith(secureKeyboard: v)),
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
          ListTile(
            key: const Key('update-frequency'),
            leading: const Icon(Icons.update),
            title: const Text('Look for updates'),
            subtitle: Text(
              '${updateCheckLabel(s.updateCheck)}. Asks GitHub for the '
              'latest release; nothing about your vaults is sent.',
            ),
            onTap: () async {
              final c = await showDialog<UpdateCheck>(
                context: context,
                builder: (ctx) => SimpleDialog(
                  title: const Text('Look for updates'),
                  children: [
                    for (final c in UpdateCheck.values)
                      RadioListTile<UpdateCheck>(
                        key: Key('update-${c.name}'),
                        value: c,
                        // ignore: deprecated_member_use
                        groupValue: s.updateCheck,
                        title: Text(updateCheckLabel(c)),
                        // ignore: deprecated_member_use
                        onChanged: (v) => Navigator.pop(ctx, v),
                      ),
                  ],
                ),
              );
              if (c != null) {
                await app.updateSettings(s.copyWith(updateCheck: c));
              }
            },
          ),
          if (s.skippedVersion.isNotEmpty)
            ListTile(
              leading: const Icon(Icons.skip_next_outlined),
              title: Text('Skipping version ${s.skippedVersion}'),
              subtitle: const Text('You are not reminded about it.'),
              trailing: TextButton(
                onPressed: () =>
                    app.updateSettings(s.copyWith(skippedVersion: '')),
                child: const Text('Undo'),
              ),
            ),
          const _Header('Packages and files'),
          SwitchListTile(
            key: const Key('remember-packages'),
            secondary: const Icon(Icons.history),
            title: const Text('Remember packages I open'),
            subtitle: const Text(
              'Lists them on the home screen by file name. They still need '
              'their passphrase or vault each time. Turning this off clears '
              'the list.',
            ),
            value: s.rememberPackages,
            onChanged: app.setRememberPackages,
          ),
          ListTile(
            key: const Key('open-as-rules'),
            leading: const Icon(Icons.file_open_outlined),
            title: const Text('File types opened another way'),
            subtitle: Text(
              s.openAsRules.isEmpty
                  ? 'None. Use "Open as" on a file in a package to choose.'
                  : s.openAsRules
                        .map((r) => '.${r.extension_} as ${r.kind.name}')
                        .join(', '),
            ),
            onTap: s.openAsRules.isEmpty
                ? null
                : () async {
                    final ok = await confirm(
                      context,
                      title: 'Forget these choices?',
                      message:
                          'Files in packages open by their type again. '
                          'Choices made for files inside a vault are kept '
                          'in that vault.',
                      action: 'Forget',
                    );
                    if (ok) {
                      await app.updateSettings(s.copyWith(openAsRules: []));
                    }
                  },
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
