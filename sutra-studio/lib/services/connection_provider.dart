import 'dart:async';
import 'package:flutter/foundation.dart';
import 'package:shared_preferences/shared_preferences.dart';
import '../models/connection_config.dart';
import 'sutra_client.dart';

/// Manages the SutraDB connection state and periodic health checks.
/// Persists connection settings via shared_preferences.
class ConnectionProvider extends ChangeNotifier {
  SutraClient _client;
  ConnectionConfig _config;
  bool _connected = false;
  String _statusMessage = 'Not connected';
  Timer? _healthTimer;
  DbStats? _stats;

  ConnectionProvider()
      : _config = const ConnectionConfig(),
        _client = SutraClient() {
    _loadSavedConfig();
  }

  /// Load saved connection config from shared_preferences.
  Future<void> _loadSavedConfig() async {
    try {
      final prefs = await SharedPreferences.getInstance();
      final endpoint = prefs.getString('sutra_endpoint');
      final apiKey = prefs.getString('sutra_api_key');
      if (endpoint != null && endpoint.isNotEmpty) {
        final saved = ConnectionConfig(
          endpoint: endpoint,
          apiKey: apiKey,
          authMethod: apiKey != null ? AuthMethod.apiKey : AuthMethod.none,
        );
        await connect(saved);
      }
    } catch (_) {
      // Ignore errors loading saved config
    }
  }

  /// Save connection config to shared_preferences.
  Future<void> _saveConfig() async {
    try {
      final prefs = await SharedPreferences.getInstance();
      await prefs.setString('sutra_endpoint', _config.endpoint);
      if (_config.apiKey != null) {
        await prefs.setString('sutra_api_key', _config.apiKey!);
      } else {
        await prefs.remove('sutra_api_key');
      }
    } catch (_) {}
  }

  SutraClient get client => _client;
  ConnectionConfig get config => _config;
  bool get connected => _connected;
  String get statusMessage => _statusMessage;
  DbStats? get stats => _stats;

  /// Update connection settings and reconnect.
  Future<void> connect(ConnectionConfig newConfig) async {
    _config = newConfig;
    _client.dispose();
    _client = SutraClient(config: _config);
    await _checkHealth();
    _startHealthCheck();
    await _saveConfig();
    notifyListeners();
  }

  /// Quick reconnect with current settings.
  Future<void> reconnect() async {
    await _checkHealth();
    notifyListeners();
  }

  Future<void> _checkHealth() async {
    try {
      _connected = await _client.health();
      _statusMessage = _connected ? 'Connected' : 'Server unreachable';
      if (_connected) {
        _stats = await _client.stats();
      }
    } catch (e) {
      _connected = false;
      _statusMessage = 'Error: $e';
    }
  }

  void _startHealthCheck() {
    _healthTimer?.cancel();
    _healthTimer = Timer.periodic(
      const Duration(seconds: 15),
      (_) async {
        await _checkHealth();
        notifyListeners();
      },
    );
  }

  @override
  void dispose() {
    _healthTimer?.cancel();
    _client.dispose();
    super.dispose();
  }
}
