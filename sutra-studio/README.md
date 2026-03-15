# Sutra Studio

Visual database management client for SutraDB — like MongoDB Compass, but for RDF-star triplestores with HNSW vector indexes.

Built with Flutter for desktop (macOS, Windows, Linux), web, and mobile.

## Why a visual client?

Most SutraDB usage happens through AI agents and application SDKs. The visual client exists for things humans are better at:

- **Spotting broken HNSW clusters** — degree distribution drift, tombstone accumulation
- **Visual graph intuition** — patterns in the knowledge graph that aren't obvious from queries
- **Quick debugging** — ad-hoc SPARQL, triple inspection, connection testing
- **Ontology browsing** — lightweight Protégé-like class hierarchy viewer

## Screens

| Screen | Purpose |
|--------|---------|
| **Health** | Database health dashboard — connection status, HNSW index diagnostics, rebuild recommendations |
| **Graph** | Force-directed graph visualization with semantic/vector/all view modes |
| **Triples** | Sortable, filterable triple table with add/delete (form + raw N-Triples) |
| **SPARQL** | Query editor with quick templates and results table |
| **Ontology** | OWL class hierarchy browser — classes, properties, individuals |
| **Auth** | Connection & authentication settings (API key / basic auth) |

## Running

```bash
cd sutra-studio
flutter pub get
flutter run -d linux    # or: macos, windows, chrome
```

## Requirements

- Flutter SDK >= 3.2.0
- A running SutraDB instance (default: `http://localhost:3030`)

## Architecture

```
lib/
├── main.dart                  # App entry, navigation shell
├── models/
│   ├── triple.dart            # RDF triple model, classification
│   ├── graph_node.dart        # Graph node/edge models for visualization
│   └── connection_config.dart # Connection & auth configuration
├── services/
│   ├── sutra_client.dart      # Dart HTTP client (mirrors TS SDK)
│   └── connection_provider.dart # Connection state management
├── screens/
│   ├── health_screen.dart     # Database health dashboard
│   ├── graph_screen.dart      # Graph visualization
│   ├── triples_screen.dart    # Triple table editor
│   ├── sparql_screen.dart     # SPARQL query editor
│   ├── ontology_screen.dart   # OWL class browser
│   └── auth_screen.dart       # Auth settings
├── widgets/
│   └── graph_canvas.dart      # Force-directed graph renderer
└── theme/
    └── sutra_theme.dart       # Dark theme (matches tools/browse.html)
```
