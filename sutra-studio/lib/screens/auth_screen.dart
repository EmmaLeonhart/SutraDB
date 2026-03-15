import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../models/connection_config.dart';
import '../services/connection_provider.dart';
import '../theme/sutra_theme.dart';

/// Authentication and connection settings screen.
///
/// SutraDB authentication is not yet implemented on the server side
/// (see TODO.md Priority 8). This screen is ready for when it is —
/// it supports API key auth, basic auth, and no-auth modes.
class AuthScreen extends StatefulWidget {
  const AuthScreen({super.key});

  @override
  State<AuthScreen> createState() => _AuthScreenState();
}

class _AuthScreenState extends State<AuthScreen> {
  late TextEditingController _endpointCtrl;
  late TextEditingController _apiKeyCtrl;
  late TextEditingController _usernameCtrl;
  late TextEditingController _passwordCtrl;
  AuthMethod _authMethod = AuthMethod.none;
  bool _testing = false;
  bool? _testResult;
  bool _obscureKey = true;
  bool _obscurePassword = true;

  @override
  void initState() {
    super.initState();
    final config = context.read<ConnectionProvider>().config;
    _endpointCtrl = TextEditingController(text: config.endpoint);
    _apiKeyCtrl = TextEditingController(text: config.apiKey ?? '');
    _usernameCtrl = TextEditingController(text: config.username ?? '');
    _passwordCtrl = TextEditingController(text: config.password ?? '');
    _authMethod = config.authMethod;
  }

  Future<void> _testConnection() async {
    setState(() {
      _testing = true;
      _testResult = null;
    });

    final conn = context.read<ConnectionProvider>();
    await conn.connect(_buildConfig());

    setState(() {
      _testing = false;
      _testResult = conn.connected;
    });
  }

  Future<void> _save() async {
    final conn = context.read<ConnectionProvider>();
    await conn.connect(_buildConfig());
    if (mounted) {
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(
          content: Text(conn.connected
              ? 'Connected successfully'
              : 'Connection failed — settings saved anyway'),
          backgroundColor:
              conn.connected ? SutraTheme.green : SutraTheme.orange,
        ),
      );
    }
  }

  ConnectionConfig _buildConfig() {
    return ConnectionConfig(
      endpoint: _endpointCtrl.text.trim(),
      apiKey: _apiKeyCtrl.text.trim().isEmpty ? null : _apiKeyCtrl.text.trim(),
      username: _usernameCtrl.text.trim().isEmpty
          ? null
          : _usernameCtrl.text.trim(),
      password: _passwordCtrl.text.trim().isEmpty
          ? null
          : _passwordCtrl.text.trim(),
      authMethod: _authMethod,
    );
  }

  @override
  Widget build(BuildContext context) {
    return SingleChildScrollView(
      padding: const EdgeInsets.all(24),
      child: Center(
        child: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 600),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              // Header
              Row(
                children: [
                  const Icon(Icons.lock_outline,
                      color: SutraTheme.accent, size: 24),
                  const SizedBox(width: 10),
                  const Text(
                    'Connection & Authentication',
                    style: TextStyle(
                      fontSize: 18,
                      fontWeight: FontWeight.w600,
                      color: SutraTheme.text,
                    ),
                  ),
                ],
              ),
              const SizedBox(height: 8),
              Container(
                padding: const EdgeInsets.all(12),
                decoration: BoxDecoration(
                  color: SutraTheme.orange.withOpacity(0.1),
                  border: Border.all(
                      color: SutraTheme.orange.withOpacity(0.3)),
                  borderRadius: BorderRadius.circular(6),
                ),
                child: const Row(
                  children: [
                    Icon(Icons.info_outline,
                        color: SutraTheme.orange, size: 16),
                    SizedBox(width: 8),
                    Expanded(
                      child: Text(
                        'Authentication is not yet implemented on the SutraDB server. '
                        'These settings are ready for when API key / basic auth support '
                        'is added (Priority 8 in the roadmap).',
                        style: TextStyle(
                            color: SutraTheme.orange, fontSize: 12),
                      ),
                    ),
                  ],
                ),
              ),

              const SizedBox(height: 24),

              // Endpoint
              const Text('Server Endpoint',
                  style: TextStyle(
                      color: SutraTheme.text,
                      fontWeight: FontWeight.w600,
                      fontSize: 13)),
              const SizedBox(height: 8),
              TextField(
                controller: _endpointCtrl,
                decoration: const InputDecoration(
                  hintText: 'http://localhost:3030',
                  prefixIcon: Icon(Icons.dns_outlined, size: 18),
                ),
              ),

              const SizedBox(height: 24),

              // Auth method
              const Text('Authentication Method',
                  style: TextStyle(
                      color: SutraTheme.text,
                      fontWeight: FontWeight.w600,
                      fontSize: 13)),
              const SizedBox(height: 8),
              SegmentedButton<AuthMethod>(
                segments: const [
                  ButtonSegment(
                    value: AuthMethod.none,
                    label: Text('None'),
                    icon: Icon(Icons.lock_open, size: 16),
                  ),
                  ButtonSegment(
                    value: AuthMethod.apiKey,
                    label: Text('API Key'),
                    icon: Icon(Icons.key, size: 16),
                  ),
                  ButtonSegment(
                    value: AuthMethod.basicAuth,
                    label: Text('Basic Auth'),
                    icon: Icon(Icons.person, size: 16),
                  ),
                ],
                selected: {_authMethod},
                onSelectionChanged: (s) =>
                    setState(() => _authMethod = s.first),
              ),

              const SizedBox(height: 16),

              // Auth fields
              if (_authMethod == AuthMethod.apiKey) ...[
                const Text('API Key',
                    style: TextStyle(
                        color: SutraTheme.text,
                        fontWeight: FontWeight.w600,
                        fontSize: 13)),
                const SizedBox(height: 8),
                TextField(
                  controller: _apiKeyCtrl,
                  obscureText: _obscureKey,
                  decoration: InputDecoration(
                    hintText: 'sutra_key_...',
                    prefixIcon: const Icon(Icons.key, size: 18),
                    suffixIcon: IconButton(
                      icon: Icon(
                        _obscureKey
                            ? Icons.visibility
                            : Icons.visibility_off,
                        size: 18,
                      ),
                      onPressed: () =>
                          setState(() => _obscureKey = !_obscureKey),
                    ),
                  ),
                ),
              ],

              if (_authMethod == AuthMethod.basicAuth) ...[
                const Text('Username',
                    style: TextStyle(
                        color: SutraTheme.text,
                        fontWeight: FontWeight.w600,
                        fontSize: 13)),
                const SizedBox(height: 8),
                TextField(
                  controller: _usernameCtrl,
                  decoration: const InputDecoration(
                    hintText: 'admin',
                    prefixIcon: Icon(Icons.person_outline, size: 18),
                  ),
                ),
                const SizedBox(height: 16),
                const Text('Password',
                    style: TextStyle(
                        color: SutraTheme.text,
                        fontWeight: FontWeight.w600,
                        fontSize: 13)),
                const SizedBox(height: 8),
                TextField(
                  controller: _passwordCtrl,
                  obscureText: _obscurePassword,
                  decoration: InputDecoration(
                    hintText: 'password',
                    prefixIcon:
                        const Icon(Icons.lock_outline, size: 18),
                    suffixIcon: IconButton(
                      icon: Icon(
                        _obscurePassword
                            ? Icons.visibility
                            : Icons.visibility_off,
                        size: 18,
                      ),
                      onPressed: () => setState(
                          () => _obscurePassword = !_obscurePassword),
                    ),
                  ),
                ),
              ],

              const SizedBox(height: 32),

              // Actions
              Row(
                children: [
                  OutlinedButton.icon(
                    onPressed: _testing ? null : _testConnection,
                    icon: _testing
                        ? const SizedBox(
                            width: 14,
                            height: 14,
                            child: CircularProgressIndicator(
                                strokeWidth: 2))
                        : const Icon(Icons.wifi_tethering, size: 16),
                    label: const Text('Test Connection'),
                  ),
                  const SizedBox(width: 8),
                  if (_testResult != null) ...[
                    Icon(
                      _testResult! ? Icons.check_circle : Icons.cancel,
                      size: 18,
                      color: _testResult!
                          ? SutraTheme.green
                          : SutraTheme.red,
                    ),
                    const SizedBox(width: 4),
                    Text(
                      _testResult! ? 'Success' : 'Failed',
                      style: TextStyle(
                        color: _testResult!
                            ? SutraTheme.green
                            : SutraTheme.red,
                        fontSize: 12,
                      ),
                    ),
                  ],
                  const Spacer(),
                  ElevatedButton.icon(
                    onPressed: _save,
                    icon: const Icon(Icons.save, size: 16),
                    label: const Text('Save & Connect'),
                  ),
                ],
              ),

              const SizedBox(height: 32),

              // Connection info
              Consumer<ConnectionProvider>(
                builder: (ctx, conn, _) => Container(
                  padding: const EdgeInsets.all(12),
                  decoration: BoxDecoration(
                    color: SutraTheme.surface,
                    border: Border.all(color: SutraTheme.border),
                    borderRadius: BorderRadius.circular(6),
                  ),
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Row(
                        children: [
                          Icon(
                            Icons.circle,
                            size: 8,
                            color: conn.connected
                                ? SutraTheme.green
                                : SutraTheme.red,
                          ),
                          const SizedBox(width: 8),
                          Text(
                            conn.statusMessage,
                            style: TextStyle(
                              color: conn.connected
                                  ? SutraTheme.green
                                  : SutraTheme.red,
                              fontSize: 13,
                            ),
                          ),
                        ],
                      ),
                      if (conn.stats != null &&
                          conn.stats!.totalTriples >= 0) ...[
                        const SizedBox(height: 8),
                        Text(
                          'Total triples: ${conn.stats!.totalTriples}',
                          style: const TextStyle(
                              color: SutraTheme.muted, fontSize: 12),
                        ),
                      ],
                    ],
                  ),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }

  @override
  void dispose() {
    _endpointCtrl.dispose();
    _apiKeyCtrl.dispose();
    _usernameCtrl.dispose();
    _passwordCtrl.dispose();
    super.dispose();
  }
}
