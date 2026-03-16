import 'dart:math';
import 'package:flutter/material.dart';
import '../models/graph_node.dart';
import '../theme/sutra_theme.dart';

/// View mode for filtering what's displayed on the graph.
enum GraphViewMode { all, semanticOnly, vectorOnly }

/// Custom painter for the force-directed graph visualization.
///
/// This is the core rendering engine — it draws nodes and edges on a Canvas,
/// handling the visual representation while [GraphCanvas] manages interaction
/// and physics simulation.
class GraphPainter extends CustomPainter {
  final List<GraphNode> nodes;
  final List<GraphEdge> edges;
  final Map<String, GraphNode> nodeMap;
  final String? selectedNodeId;
  final GraphViewMode viewMode;
  final Offset panOffset;
  final double zoom;

  GraphPainter({
    required this.nodes,
    required this.edges,
    required this.nodeMap,
    this.selectedNodeId,
    this.viewMode = GraphViewMode.all,
    this.panOffset = Offset.zero,
    this.zoom = 1.0,
  });

  @override
  void paint(Canvas canvas, Size size) {
    canvas.save();
    canvas.translate(
        size.width / 2 + panOffset.dx, size.height / 2 + panOffset.dy);
    canvas.scale(zoom);

    // Draw edges
    for (final edge in edges) {
      if (!_edgeVisible(edge)) continue;
      final source = nodeMap[edge.sourceId];
      final target = nodeMap[edge.targetId];
      if (source == null || target == null) continue;

      final paint = Paint()
        ..strokeWidth = 1.0
        ..style = PaintingStyle.stroke;

      switch (edge.type) {
        case EdgeType.semantic:
          paint.color = SutraTheme.edgeSemantic;
        case EdgeType.vector:
          paint.color = SutraTheme.edgeVector;
        case EdgeType.hnswNeighbor:
          paint.color = SutraTheme.edgeHnsw;
          paint.strokeWidth = 0.5;
      }

      canvas.drawLine(source.position, target.position, paint);

      // Arrowhead
      _drawArrow(canvas, source.position, target.position, paint,
          target.radius);

      // Edge label (only if zoomed in enough)
      if (zoom > 0.6 && edge.label.isNotEmpty) {
        final mid = Offset(
          (source.position.dx + target.position.dx) / 2,
          (source.position.dy + target.position.dy) / 2,
        );
        final tp = TextPainter(
          text: TextSpan(
            text: edge.label,
            style: TextStyle(
              color: SutraTheme.muted.withOpacity(0.7),
              fontSize: 9 / zoom,
            ),
          ),
          textDirection: TextDirection.ltr,
        )..layout();
        tp.paint(canvas, mid - Offset(tp.width / 2, tp.height / 2));
      }
    }

    // Draw nodes
    for (final node in nodes) {
      if (!_nodeVisible(node)) continue;
      final isSelected = node.id == selectedNodeId;

      Color color;
      switch (node.type) {
        case NodeType.entity:
          color = SutraTheme.nodeEntity;
        case NodeType.literal:
          color = SutraTheme.nodeLiteral;
        case NodeType.blankNode:
          color = SutraTheme.nodeBlank;
        case NodeType.vectorLiteral:
          color = SutraTheme.nodeVector;
      }

      // Glow for selected node
      if (isSelected) {
        final glow = Paint()
          ..color = color.withOpacity(0.3)
          ..maskFilter = const MaskFilter.blur(BlurStyle.normal, 8);
        canvas.drawCircle(node.position, node.radius + 4, glow);
      }

      // Node circle
      final fill = Paint()..color = color;
      canvas.drawCircle(node.position, node.radius, fill);

      // Vector indicator ring
      if (node.hasVector) {
        final ring = Paint()
          ..color = SutraTheme.purple
          ..style = PaintingStyle.stroke
          ..strokeWidth = 2;
        canvas.drawCircle(node.position, node.radius + 3, ring);
      }

      // Border
      final border = Paint()
        ..color = isSelected ? Colors.white : color.withOpacity(0.6)
        ..style = PaintingStyle.stroke
        ..strokeWidth = isSelected ? 2 : 1;
      canvas.drawCircle(node.position, node.radius, border);

      // Label
      if (zoom > 0.4) {
        final tp = TextPainter(
          text: TextSpan(
            text: node.label,
            style: TextStyle(
              color: SutraTheme.text,
              fontSize: 10 / zoom,
              fontWeight: isSelected ? FontWeight.w600 : FontWeight.normal,
            ),
          ),
          textDirection: TextDirection.ltr,
        )..layout(maxWidth: 120 / zoom);
        tp.paint(
          canvas,
          node.position + Offset(-tp.width / 2, node.radius + 4),
        );
      }
    }

    canvas.restore();
  }

  void _drawArrow(
      Canvas canvas, Offset from, Offset to, Paint paint, double targetR) {
    final dx = to.dx - from.dx;
    final dy = to.dy - from.dy;
    final len = sqrt(dx * dx + dy * dy);
    if (len < 1) return;
    final ux = dx / len;
    final uy = dy / len;
    final tip = Offset(to.dx - ux * targetR, to.dy - uy * targetR);
    const arrowLen = 8.0;
    const arrowAngle = 0.4;
    final p1 = Offset(
      tip.dx - arrowLen * (ux * cos(arrowAngle) - uy * sin(arrowAngle)),
      tip.dy - arrowLen * (uy * cos(arrowAngle) + ux * sin(arrowAngle)),
    );
    final p2 = Offset(
      tip.dx - arrowLen * (ux * cos(arrowAngle) + uy * sin(arrowAngle)),
      tip.dy - arrowLen * (uy * cos(arrowAngle) - ux * sin(arrowAngle)),
    );
    final path = Path()
      ..moveTo(tip.dx, tip.dy)
      ..lineTo(p1.dx, p1.dy)
      ..lineTo(p2.dx, p2.dy)
      ..close();
    canvas.drawPath(
        path,
        Paint()
          ..color = paint.color
          ..style = PaintingStyle.fill);
  }

  bool _edgeVisible(GraphEdge edge) {
    switch (viewMode) {
      case GraphViewMode.all:
        return true;
      case GraphViewMode.semanticOnly:
        return edge.type == EdgeType.semantic;
      case GraphViewMode.vectorOnly:
        return edge.type == EdgeType.vector ||
            edge.type == EdgeType.hnswNeighbor;
    }
  }

  bool _nodeVisible(GraphNode node) {
    switch (viewMode) {
      case GraphViewMode.all:
        return true;
      case GraphViewMode.semanticOnly:
        return node.type != NodeType.vectorLiteral;
      case GraphViewMode.vectorOnly:
        return node.hasVector || node.type == NodeType.vectorLiteral;
    }
  }

  @override
  bool shouldRepaint(covariant GraphPainter old) => true;
}

/// Interactive graph visualization widget with force-directed layout.
class GraphCanvas extends StatefulWidget {
  final List<GraphNode> nodes;
  final List<GraphEdge> edges;
  final GraphViewMode viewMode;
  final ValueChanged<String?>? onNodeSelected;
  final ValueChanged<String>? onNodeDoubleTap;

  const GraphCanvas({
    super.key,
    required this.nodes,
    required this.edges,
    this.viewMode = GraphViewMode.all,
    this.onNodeSelected,
    this.onNodeDoubleTap,
  });

  @override
  State<GraphCanvas> createState() => _GraphCanvasState();
}

class _GraphCanvasState extends State<GraphCanvas>
    with SingleTickerProviderStateMixin {
  late AnimationController _ticker;
  late Map<String, GraphNode> _nodeMap;
  String? _selectedNodeId;
  String? _draggedNodeId;
  Offset _panOffset = Offset.zero;
  double _zoom = 1.0;
  Offset? _lastPan;

  @override
  void initState() {
    super.initState();
    _nodeMap = {for (final n in widget.nodes) n.id: n};
    _initPositions();
    _ticker = AnimationController(
      vsync: this,
      duration: const Duration(seconds: 1),
    )..addListener(_simulationStep);
    _ticker.repeat();
  }

  void _initPositions() {
    final rng = Random(42);
    for (final node in widget.nodes) {
      node.position = Offset(
        (rng.nextDouble() - 0.5) * 400,
        (rng.nextDouble() - 0.5) * 400,
      );
    }
  }

  @override
  void didUpdateWidget(GraphCanvas old) {
    super.didUpdateWidget(old);
    if (widget.nodes != old.nodes) {
      _nodeMap = {for (final n in widget.nodes) n.id: n};
      _initPositions();
    }
  }

  /// Simple force-directed simulation step.
  void _simulationStep() {
    const repulsion = 2000.0;
    const attraction = 0.005;
    const damping = 0.85;
    const centerGravity = 0.01;

    // Repulsion between all pairs
    for (int i = 0; i < widget.nodes.length; i++) {
      for (int j = i + 1; j < widget.nodes.length; j++) {
        final a = widget.nodes[i];
        final b = widget.nodes[j];
        var dx = a.position.dx - b.position.dx;
        var dy = a.position.dy - b.position.dy;
        var dist = sqrt(dx * dx + dy * dy);
        if (dist < 1) dist = 1;
        final force = repulsion / (dist * dist);
        final fx = (dx / dist) * force;
        final fy = (dy / dist) * force;
        if (!a.isDragging) a.velocity += Offset(fx, fy);
        if (!b.isDragging) b.velocity -= Offset(fx, fy);
      }
    }

    // Attraction along edges
    for (final edge in widget.edges) {
      final a = _nodeMap[edge.sourceId];
      final b = _nodeMap[edge.targetId];
      if (a == null || b == null) continue;
      final dx = b.position.dx - a.position.dx;
      final dy = b.position.dy - a.position.dy;
      final fx = dx * attraction;
      final fy = dy * attraction;
      if (!a.isDragging) a.velocity += Offset(fx, fy);
      if (!b.isDragging) b.velocity -= Offset(fx, fy);
    }

    // Center gravity + damping + apply
    for (final node in widget.nodes) {
      if (node.isDragging) continue;
      node.velocity -= Offset(
        node.position.dx * centerGravity,
        node.position.dy * centerGravity,
      );
      node.velocity = node.velocity * damping;
      node.position += node.velocity;
    }

    setState(() {});
  }

  GraphNode? _hitTest(Offset localPos) {
    final worldPos = Offset(
      (localPos.dx - _panOffset.dx) / _zoom,
      (localPos.dy - _panOffset.dy) / _zoom,
    );
    // Adjust for center-origin
    // The painter translates by size/2, but we don't have size here.
    // We'll approximate with the render box.
    final box = context.findRenderObject() as RenderBox?;
    if (box == null) return null;
    final center = box.size.center(Offset.zero);
    final adjusted = Offset(
      (localPos.dx - center.dx - _panOffset.dx) / _zoom,
      (localPos.dy - center.dy - _panOffset.dy) / _zoom,
    );

    for (final node in widget.nodes.reversed) {
      final d = (node.position - adjusted).distance;
      if (d <= node.radius + 4) return node;
    }
    return null;
  }

  @override
  Widget build(BuildContext context) {
    return GestureDetector(
      onScaleStart: (d) {
        final hit = _hitTest(d.localFocalPoint);
        if (hit != null) {
          _draggedNodeId = hit.id;
          hit.isDragging = true;
          _selectedNodeId = hit.id;
          widget.onNodeSelected?.call(hit.id);
        }
        _lastPan = d.localFocalPoint;
      },
      onScaleUpdate: (d) {
        if (_draggedNodeId != null) {
          final node = _nodeMap[_draggedNodeId];
          if (node != null) {
            final box = context.findRenderObject() as RenderBox?;
            final center = box?.size.center(Offset.zero) ?? Offset.zero;
            node.position = Offset(
              (d.localFocalPoint.dx - center.dx - _panOffset.dx) / _zoom,
              (d.localFocalPoint.dy - center.dy - _panOffset.dy) / _zoom,
            );
            node.velocity = Offset.zero;
          }
        } else if (_lastPan != null) {
          _panOffset += d.localFocalPoint - _lastPan!;
        }
        _lastPan = d.localFocalPoint;
        if (d.scale != 1.0) {
          _zoom = (_zoom * d.scale).clamp(0.1, 5.0);
        }
      },
      onScaleEnd: (_) {
        if (_draggedNodeId != null) {
          _nodeMap[_draggedNodeId]?.isDragging = false;
          _draggedNodeId = null;
        }
        _lastPan = null;
      },
      onTapUp: (d) {
        final hit = _hitTest(d.localPosition);
        setState(() => _selectedNodeId = hit?.id);
        widget.onNodeSelected?.call(hit?.id);
      },
      onDoubleTapDown: (d) {
        final hit = _hitTest(d.localPosition);
        if (hit != null) {
          widget.onNodeDoubleTap?.call(hit.id);
        }
      },
      child: ClipRect(
        child: CustomPaint(
          painter: GraphPainter(
            nodes: widget.nodes,
            edges: widget.edges,
            nodeMap: _nodeMap,
            selectedNodeId: _selectedNodeId,
            viewMode: widget.viewMode,
            panOffset: _panOffset,
            zoom: _zoom,
          ),
          size: Size.infinite,
        ),
      ),
    );
  }

  @override
  void dispose() {
    _ticker.dispose();
    super.dispose();
  }
}
