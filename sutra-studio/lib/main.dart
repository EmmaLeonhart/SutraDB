import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import 'services/connection_provider.dart';
import 'screens/graph_screen.dart';
import 'screens/triples_screen.dart';
import 'screens/sparql_screen.dart';
import 'screens/ontology_screen.dart';
import 'screens/auth_screen.dart';
import 'screens/health_screen.dart';
import 'theme/sutra_theme.dart';

void main() {
  runApp(const SutraStudioApp());
}

class SutraStudioApp extends StatefulWidget {
  const SutraStudioApp({super.key});

  @override
  State<SutraStudioApp> createState() => _SutraStudioAppState();

  /// Access theme toggle from anywhere.
  static _SutraStudioAppState? of(BuildContext context) =>
      context.findAncestorStateOfType<_SutraStudioAppState>();
}

class _SutraStudioAppState extends State<SutraStudioApp> {
  ThemeMode _themeMode = ThemeMode.dark;

  void toggleTheme() {
    setState(() {
      _themeMode =
          _themeMode == ThemeMode.dark ? ThemeMode.light : ThemeMode.dark;
    });
  }

  ThemeMode get themeMode => _themeMode;

  @override
  Widget build(BuildContext context) {
    return ChangeNotifierProvider(
      create: (_) => ConnectionProvider(),
      child: MaterialApp(
        title: 'Sutra Studio',
        debugShowCheckedModeBanner: false,
        theme: ThemeData.light(useMaterial3: true),
        darkTheme: SutraTheme.darkTheme,
        themeMode: _themeMode,
        home: const MainShell(),
      ),
    );
  }
}

/// Main navigation shell with a left navigation rail.
class MainShell extends StatefulWidget {
  const MainShell({super.key});

  @override
  State<MainShell> createState() => _MainShellState();
}

class _MainShellState extends State<MainShell> {
  int _selectedIndex = 0;

  static const _screens = <Widget>[
    HealthScreen(),
    GraphScreen(),
    TriplesScreen(),
    SparqlScreen(),
    OntologyScreen(),
    AuthScreen(),
  ];

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      body: Row(
        children: [
          // Navigation rail
          NavigationRail(
            selectedIndex: _selectedIndex,
            onDestinationSelected: (i) =>
                setState(() => _selectedIndex = i),
            labelType: NavigationRailLabelType.all,
            leading: Padding(
              padding: const EdgeInsets.symmetric(vertical: 12),
              child: Column(
                children: [
                  const Text(
                    'Sutra',
                    style: TextStyle(
                      fontWeight: FontWeight.w700,
                      color: SutraTheme.accent,
                      fontSize: 14,
                    ),
                  ),
                  const Text(
                    'Studio',
                    style: TextStyle(
                      color: SutraTheme.muted,
                      fontSize: 10,
                    ),
                  ),
                  const SizedBox(height: 8),
                  // Theme toggle
                  IconButton(
                    icon: Icon(
                      SutraStudioApp.of(context)?.themeMode == ThemeMode.dark
                          ? Icons.light_mode
                          : Icons.dark_mode,
                      size: 14,
                      color: SutraTheme.muted,
                    ),
                    onPressed: () => SutraStudioApp.of(context)?.toggleTheme(),
                    tooltip: 'Toggle theme',
                    iconSize: 14,
                    padding: EdgeInsets.zero,
                    constraints: const BoxConstraints(minWidth: 24, minHeight: 24),
                  ),
                  const SizedBox(height: 4),
                  // Connection indicator
                  Consumer<ConnectionProvider>(
                    builder: (ctx, conn, _) => Tooltip(
                      message: conn.statusMessage,
                      child: Icon(
                        Icons.circle,
                        size: 8,
                        color: conn.connected
                            ? SutraTheme.green
                            : SutraTheme.red,
                      ),
                    ),
                  ),
                ],
              ),
            ),
            destinations: const [
              NavigationRailDestination(
                icon: Icon(Icons.monitor_heart_outlined, size: 20),
                selectedIcon: Icon(Icons.monitor_heart, size: 20),
                label: Text('Health'),
              ),
              NavigationRailDestination(
                icon: Icon(Icons.hub_outlined, size: 20),
                selectedIcon: Icon(Icons.hub, size: 20),
                label: Text('Graph'),
              ),
              NavigationRailDestination(
                icon: Icon(Icons.table_rows_outlined, size: 20),
                selectedIcon: Icon(Icons.table_rows, size: 20),
                label: Text('Triples'),
              ),
              NavigationRailDestination(
                icon: Icon(Icons.code, size: 20),
                selectedIcon: Icon(Icons.code, size: 20),
                label: Text('SPARQL'),
              ),
              NavigationRailDestination(
                icon: Icon(Icons.account_tree_outlined, size: 20),
                selectedIcon: Icon(Icons.account_tree, size: 20),
                label: Text('Ontology'),
              ),
              NavigationRailDestination(
                icon: Icon(Icons.lock_outline, size: 20),
                selectedIcon: Icon(Icons.lock, size: 20),
                label: Text('Auth'),
              ),
            ],
          ),
          const VerticalDivider(width: 1, color: SutraTheme.border),
          // Content
          Expanded(child: _screens[_selectedIndex]),
        ],
      ),
    );
  }
}
