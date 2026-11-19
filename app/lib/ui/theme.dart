import 'package:flutter/material.dart';

/// Earthy palette drawn from the radish logo: leaf greens, soil browns and
/// radish pinks on warm cream (light) or dark soil (dark).
abstract final class AtroposTheme {
  // Night indigo with a brass accent: Atropos keeps things under lock.
  static final _light = ColorScheme.fromSeed(
    seedColor: const Color(0xFF34407A),
    brightness: Brightness.light,
    tertiary: const Color(0xFF8A6A1F),
  ).copyWith(surfaceTint: Colors.transparent);

  static final _dark = ColorScheme.fromSeed(
    seedColor: const Color(0xFF34407A),
    brightness: Brightness.dark,
    tertiary: const Color(0xFFE2C26A),
  ).copyWith(surfaceTint: Colors.transparent);

  static ThemeData get light => _build(_light);
  static ThemeData get dark => _build(_dark);

  static ThemeData _build(ColorScheme s) {
    final rounded = RoundedRectangleBorder(
      borderRadius: BorderRadius.circular(12),
    );
    return ThemeData(
      useMaterial3: true,
      colorScheme: s,
      scaffoldBackgroundColor: s.surface,
      appBarTheme: AppBarTheme(
        backgroundColor: s.surface,
        foregroundColor: s.onSurface,
        surfaceTintColor: Colors.transparent,
        scrolledUnderElevation: 0,
        elevation: 0,
        centerTitle: false,
        shape: Border(bottom: BorderSide(color: s.outlineVariant)),
      ),
      cardTheme: CardThemeData(
        color: s.surfaceContainerLow,
        surfaceTintColor: Colors.transparent,
        elevation: 0,
        margin: const EdgeInsets.symmetric(vertical: 4),
        shape: RoundedRectangleBorder(
          borderRadius: BorderRadius.circular(16),
          side: BorderSide(color: s.outlineVariant),
        ),
      ),
      dialogTheme: DialogThemeData(
        backgroundColor: s.surfaceContainerHigh,
        surfaceTintColor: Colors.transparent,
        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(20)),
      ),
      filledButtonTheme: FilledButtonThemeData(
        style: FilledButton.styleFrom(shape: rounded),
      ),
      outlinedButtonTheme: OutlinedButtonThemeData(
        style: OutlinedButton.styleFrom(
          shape: rounded,
          side: BorderSide(color: s.outline),
        ),
      ),
      textButtonTheme: TextButtonThemeData(
        style: TextButton.styleFrom(shape: rounded),
      ),
      floatingActionButtonTheme: FloatingActionButtonThemeData(
        backgroundColor: s.primaryContainer,
        foregroundColor: s.onPrimaryContainer,
        elevation: 1,
        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(16)),
      ),
      inputDecorationTheme: InputDecorationTheme(
        filled: true,
        fillColor: s.surfaceContainer,
        border: OutlineInputBorder(
          borderRadius: BorderRadius.circular(12),
          borderSide: BorderSide(color: s.outlineVariant),
        ),
        enabledBorder: OutlineInputBorder(
          borderRadius: BorderRadius.circular(12),
          borderSide: BorderSide(color: s.outlineVariant),
        ),
        focusedBorder: OutlineInputBorder(
          borderRadius: BorderRadius.circular(12),
          borderSide: BorderSide(color: s.primary, width: 2),
        ),
      ),
      chipTheme: ChipThemeData(
        backgroundColor: s.surfaceContainer,
        selectedColor: s.secondaryContainer,
        side: BorderSide(color: s.outlineVariant),
        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(10)),
      ),
      tabBarTheme: TabBarThemeData(
        labelColor: s.primary,
        unselectedLabelColor: s.onSurfaceVariant,
        indicatorColor: s.primary,
        dividerColor: s.outlineVariant,
      ),
      navigationBarTheme: NavigationBarThemeData(
        backgroundColor: s.surfaceContainer,
        indicatorColor: s.secondaryContainer,
        surfaceTintColor: Colors.transparent,
      ),
      listTileTheme: ListTileThemeData(iconColor: s.secondary, shape: rounded),
      dividerTheme: DividerThemeData(color: s.outlineVariant, space: 1),
      snackBarTheme: SnackBarThemeData(
        behavior: SnackBarBehavior.floating,
        backgroundColor: s.inverseSurface,
        contentTextStyle: TextStyle(color: s.onInverseSurface),
        actionTextColor: s.inversePrimary,
        shape: rounded,
      ),
      progressIndicatorTheme: ProgressIndicatorThemeData(
        color: s.primary,
        linearTrackColor: s.surfaceContainerHighest,
      ),
      tooltipTheme: TooltipThemeData(
        decoration: BoxDecoration(
          color: s.inverseSurface,
          borderRadius: BorderRadius.circular(8),
        ),
        textStyle: TextStyle(color: s.onInverseSurface),
      ),
    );
  }
}
