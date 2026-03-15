import 'dart:ui';

/// A node in the graph visualization.
class GraphNode {
  final String id;
  final String label;
  final NodeType type;
  Offset position;
  Offset velocity;
  bool isDragging;

  /// Whether this node has vector embeddings attached.
  final bool hasVector;

  /// Number of connections (for sizing).
  int degree;

  GraphNode({
    required this.id,
    required this.label,
    this.type = NodeType.entity,
    Offset? position,
    this.hasVector = false,
    this.degree = 0,
  })  : position = position ?? Offset.zero,
        velocity = Offset.zero,
        isDragging = false;

  double get radius => 8.0 + (degree * 1.5).clamp(0, 20);
}

/// A directed edge in the graph visualization.
class GraphEdge {
  final String sourceId;
  final String targetId;
  final String label;
  final EdgeType type;

  const GraphEdge({
    required this.sourceId,
    required this.targetId,
    required this.label,
    this.type = EdgeType.semantic,
  });
}

enum NodeType { entity, literal, blankNode, vectorLiteral }

enum EdgeType { semantic, vector, hnswNeighbor }
