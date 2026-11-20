import '../src/rust/api/settings.dart';
import '../src/rust/api/vault.dart' show SortBy, SortOrder;
import '../src/rust/api/viewer.dart' show PreviewKind;

/// Settings are plain Rust structs; this makes changing one field easy.
extension AppSettingsCopy on AppSettings {
  AppSettings copyWith({
    int? autoLockMinutes,
    bool? secureKeyboard,
    UpdateCheck? updateCheck,
    int? lastUpdateCheck,
    String? skippedVersion,
    String? notesShownFor,
    bool? rememberPackages,
    SortOrder? packageSort,
    List<OpenAsRule>? openAsRules,
  }) => AppSettings(
    autoLockMinutes: autoLockMinutes ?? this.autoLockMinutes,
    secureKeyboard: secureKeyboard ?? this.secureKeyboard,
    updateCheck: updateCheck ?? this.updateCheck,
    lastUpdateCheck: lastUpdateCheck ?? this.lastUpdateCheck,
    skippedVersion: skippedVersion ?? this.skippedVersion,
    notesShownFor: notesShownFor ?? this.notesShownFor,
    rememberPackages: rememberPackages ?? this.rememberPackages,
    packageSort: packageSort ?? this.packageSort,
    openAsRules: openAsRules ?? this.openAsRules,
  );

  /// How packages show files ending in `.ext`, if the user picked one.
  PreviewKind? openAsFor(String name) {
    final ext = fileExtension(name);
    if (ext.isEmpty) return null;
    for (final r in openAsRules) {
      if (r.extension_ == ext) return r.kind;
    }
    return null;
  }

  /// Remembers (or with null forgets) how to show files ending in `.ext`.
  AppSettings withOpenAs(String name, PreviewKind? kind) {
    final ext = fileExtension(name);
    final rules = [
      for (final r in openAsRules)
        if (r.extension_ != ext) r,
      if (kind != null && ext.isNotEmpty)
        OpenAsRule(extension_: ext, kind: kind),
    ];
    return copyWith(openAsRules: rules);
  }
}

/// The lower-case extension without the dot ("" when there is none).
String fileExtension(String name) {
  final dot = name.lastIndexOf('.');
  if (dot <= 0 || dot == name.length - 1) return '';
  return name.substring(dot + 1).toLowerCase();
}

const defaultSettings = AppSettings(
  autoLockMinutes: 0,
  secureKeyboard: true,
  updateCheck: UpdateCheck.everyStart,
  lastUpdateCheck: 0,
  skippedVersion: '',
  notesShownFor: '',
  rememberPackages: true,
  packageSort: SortOrder(by: SortBy.name, descending: false),
  openAsRules: [],
);

String updateCheckLabel(UpdateCheck c) => switch (c) {
  UpdateCheck.off => 'Never (check by hand)',
  UpdateCheck.everyStart => 'Every time Atropos starts',
  UpdateCheck.daily => 'Once a day',
  UpdateCheck.weekly => 'Once a week',
};
