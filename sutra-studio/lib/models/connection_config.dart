/// Configuration for connecting to a SutraDB instance.
class ConnectionConfig {
  final String endpoint;
  final String? apiKey;
  final String? username;
  final String? password;
  final AuthMethod authMethod;
  final Duration timeout;

  const ConnectionConfig({
    this.endpoint = 'http://localhost:3030',
    this.apiKey,
    this.username,
    this.password,
    this.authMethod = AuthMethod.none,
    this.timeout = const Duration(seconds: 30),
  });

  ConnectionConfig copyWith({
    String? endpoint,
    String? apiKey,
    String? username,
    String? password,
    AuthMethod? authMethod,
    Duration? timeout,
  }) {
    return ConnectionConfig(
      endpoint: endpoint ?? this.endpoint,
      apiKey: apiKey ?? this.apiKey,
      username: username ?? this.username,
      password: password ?? this.password,
      authMethod: authMethod ?? this.authMethod,
      timeout: timeout ?? this.timeout,
    );
  }

  /// Build HTTP headers for authentication.
  Map<String, String> get authHeaders {
    switch (authMethod) {
      case AuthMethod.none:
        return {};
      case AuthMethod.apiKey:
        if (apiKey == null || apiKey!.isEmpty) return {};
        return {'Authorization': 'Bearer $apiKey'};
      case AuthMethod.basicAuth:
        if (username == null || password == null) return {};
        final credentials =
            Uri.encodeFull('$username:$password');
        return {'Authorization': 'Basic $credentials'};
    }
  }
}

enum AuthMethod { none, apiKey, basicAuth }
