import 'dart:async';
import 'package:flutter/foundation.dart';
import '../models/connection_config.dart';
import 'sutra_client.dart';

/// Manages the SutraDB connection state and periodic health checks.
class ConnectionProvider extends ChangeNotifier {
  SutraClient _client;
  ConnectionConfig _config;
  bool _connected = false;
  String _statusMessage = 'Not connected';
  Timer? _healthTimer;
  DbStats? _stats;

  ConnectionProvider()
      : _config = const ConnectionConfig(),
        _client = SutraClient();

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
