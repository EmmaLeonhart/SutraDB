import 'package:flutter/material.dart';

/// SutraDB Studio theme — dark theme inspired by the web browser (tools/browse.html).
class SutraTheme {
  static const bg = Color(0xFF0D1117);
  static const surface = Color(0xFF161B22);
  static const border = Color(0xFF30363D);
  static const text = Color(0xFFE6EDF3);
  static const muted = Color(0xFF8B949E);
  static const accent = Color(0xFF58A6FF);
  static const green = Color(0xFF3FB950);
  static const orange = Color(0xFFD29922);
  static const purple = Color(0xFFBC8CFF);
  static const red = Color(0xFFF85149);

  // Node colors by type
  static const nodeEntity = accent;
  static const nodeLiteral = orange;
  static const nodeBlank = muted;
  static const nodeVector = purple;

  // Edge colors by type
  static const edgeSemantic = Color(0xFF444C56);
  static const edgeVector = purple;
  static const edgeHnsw = Color(0x8858A6FF);

  static ThemeData get darkTheme => ThemeData(
        brightness: Brightness.dark,
        scaffoldBackgroundColor: bg,
        canvasColor: surface,
        cardColor: surface,
        dividerColor: border,
        primaryColor: accent,
        colorScheme: const ColorScheme.dark(
          primary: accent,
          secondary: purple,
          surface: surface,
          error: red,
          onPrimary: bg,
          onSecondary: text,
          onSurface: text,
          onError: text,
        ),
        appBarTheme: const AppBarTheme(
          backgroundColor: surface,
          foregroundColor: text,
          elevation: 0,
        ),
        navigationRailTheme: const NavigationRailThemeData(
          backgroundColor: surface,
          selectedIconTheme: IconThemeData(color: accent),
          unselectedIconTheme: IconThemeData(color: muted),
          selectedLabelTextStyle: TextStyle(color: accent, fontSize: 11),
          unselectedLabelTextStyle: TextStyle(color: muted, fontSize: 11),
        ),
        inputDecorationTheme: InputDecorationTheme(
          filled: true,
          fillColor: surface,
          border: OutlineInputBorder(
            borderSide: const BorderSide(color: border),
            borderRadius: BorderRadius.circular(6),
          ),
          enabledBorder: OutlineInputBorder(
            borderSide: const BorderSide(color: border),
            borderRadius: BorderRadius.circular(6),
          ),
          focusedBorder: OutlineInputBorder(
            borderSide: const BorderSide(color: accent),
            borderRadius: BorderRadius.circular(6),
          ),
          labelStyle: const TextStyle(color: muted),
          hintStyle: const TextStyle(color: muted),
        ),
        textTheme: const TextTheme(
          bodyLarge: TextStyle(color: text),
          bodyMedium: TextStyle(color: text),
          bodySmall: TextStyle(color: muted),
          titleLarge: TextStyle(color: text, fontWeight: FontWeight.w600),
          titleMedium: TextStyle(color: text),
          labelLarge: TextStyle(color: text),
        ),
        iconTheme: const IconThemeData(color: muted),
        chipTheme: ChipThemeData(
          backgroundColor: surface,
          selectedColor: accent.withOpacity(0.2),
          side: const BorderSide(color: border),
          labelStyle: const TextStyle(color: text, fontSize: 12),
        ),
        elevatedButtonTheme: ElevatedButtonThemeData(
          style: ElevatedButton.styleFrom(
            backgroundColor: accent,
            foregroundColor: bg,
            shape: RoundedRectangleBorder(
                borderRadius: BorderRadius.circular(6)),
          ),
        ),
        outlinedButtonTheme: OutlinedButtonThemeData(
          style: OutlinedButton.styleFrom(
            foregroundColor: accent,
            side: const BorderSide(color: border),
            shape: RoundedRectangleBorder(
                borderRadius: BorderRadius.circular(6)),
          ),
        ),
        dataTableTheme: const DataTableThemeData(
          headingTextStyle: TextStyle(
              color: text, fontWeight: FontWeight.w600, fontSize: 13),
          dataTextStyle: TextStyle(color: text, fontSize: 13),
          dividerThickness: 1,
        ),
      );
}
