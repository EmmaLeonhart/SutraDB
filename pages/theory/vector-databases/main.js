const COLORS = {
    bg: '#0a0a0f',
    grid: '#1a1a25',
    axis: '#2a2a3a',
    text: '#d0d0dc',
    textDim: '#707088',
    layer0: '#7c8cf8',
    layer1: '#facc15',
    layer2: '#f87c7c',
    query: '#34d399',
    visited: '#4ecdc4',
    edge: '#1e1e2a',
    edgeActive: '#7c8cf866',
    current: '#ffffff',
};
const LAYER_COLORS = [COLORS.layer0, COLORS.layer1, COLORS.layer2];
// --- Data: People as HNSW nodes ---
const nodes = new Map();
function addNode(id, name, x, y, layer, embedding, json) {
    nodes.set(id, { id, name, pos: { x, y }, layer, embedding, json, neighbors: new Map() });
}
function addEdge(a, b, layer) {
    const na = nodes.get(a);
    const nb = nodes.get(b);
    if (!na.neighbors.has(layer))
        na.neighbors.set(layer, []);
    if (!nb.neighbors.has(layer))
        nb.neighbors.set(layer, []);
    na.neighbors.get(layer).push(b);
    nb.neighbors.get(layer).push(a);
}
// Embeddings: names similar to "John" cluster near [0.9, 0.1]
// Query "John" = [1.0, 0.0]
addNode('john1', 'John Smith', 0.72, 0.18, 0, [0.95, 0.05], { fatherId: 'robert1' });
addNode('jean1', 'Jean Dupont', 0.68, 0.28, 0, [0.88, 0.12], { fatherId: 'pierre1' });
addNode('johan1', 'Johan Berg', 0.78, 0.12, 1, [0.92, 0.08], { fatherId: 'erik1' });
addNode('juan1', 'Juan Garcia', 0.62, 0.22, 0, [0.85, 0.15], { fatherId: 'carlos1' });
addNode('robert1', 'Robert Smith', 0.35, 0.55, 0, [0.40, 0.55], { fatherId: 'william1' });
addNode('pierre1', 'Pierre Dupont', 0.28, 0.62, 0, [0.30, 0.60], { fatherId: 'louis1' });
addNode('erik1', 'Erik Berg', 0.42, 0.48, 1, [0.45, 0.50], { fatherId: 'olaf1' });
addNode('carlos1', 'Carlos Garcia', 0.22, 0.58, 0, [0.25, 0.58], { fatherId: 'miguel1' });
addNode('william1', 'William Smith', 0.38, 0.78, 0, [0.35, 0.75], { fatherId: 'henry1' });
addNode('louis1', 'Louis Dupont', 0.18, 0.82, 0, [0.20, 0.80], {});
addNode('olaf1', 'Olaf Berg', 0.48, 0.72, 0, [0.50, 0.70], {});
addNode('miguel1', 'Miguel Garcia', 0.12, 0.75, 0, [0.15, 0.75], {});
addNode('henry1', 'Henry Smith', 0.42, 0.88, 0, [0.40, 0.88], {});
// Entry point (sparse layer 2)
addNode('alice1', 'Alice Johnson', 0.50, 0.45, 2, [0.50, 0.45], {});
addNode('bob1', 'Bob Williams', 0.30, 0.35, 2, [0.30, 0.35], {});
addNode('maria1', 'Maria Rodriguez', 0.55, 0.68, 1, [0.55, 0.65], {});
addNode('chen1', 'Chen Wei', 0.82, 0.55, 1, [0.80, 0.55], {});
// Layer 2 edges (sparse, long-range)
addEdge('alice1', 'bob1', 2);
addEdge('alice1', 'chen1', 2); // bridge down idea
// Layer 1 edges (medium)
addEdge('alice1', 'johan1', 1);
addEdge('alice1', 'maria1', 1);
addEdge('alice1', 'erik1', 1);
addEdge('bob1', 'maria1', 1);
addEdge('chen1', 'johan1', 1);
addEdge('chen1', 'maria1', 1);
addEdge('maria1', 'erik1', 1);
// Layer 0 edges (dense, short-range)
addEdge('john1', 'jean1', 0);
addEdge('john1', 'johan1', 0);
addEdge('john1', 'juan1', 0);
addEdge('jean1', 'juan1', 0);
addEdge('johan1', 'chen1', 0);
addEdge('alice1', 'robert1', 0);
addEdge('alice1', 'erik1', 0);
addEdge('bob1', 'pierre1', 0);
addEdge('bob1', 'carlos1', 0);
addEdge('maria1', 'robert1', 0);
addEdge('maria1', 'olaf1', 0);
addEdge('erik1', 'robert1', 0);
addEdge('chen1', 'john1', 0);
addEdge('robert1', 'william1', 0);
addEdge('pierre1', 'louis1', 0);
addEdge('carlos1', 'miguel1', 0);
addEdge('william1', 'henry1', 0);
addEdge('olaf1', 'william1', 0);
const queryEmbedding = [1.0, 0.0]; // "John"
function dist(a, b) {
    const dx = a[0] - b[0];
    const dy = a[1] - b[1];
    return Math.sqrt(dx * dx + dy * dy);
}
let state = {
    running: false,
    currentLayer: 2,
    currentNode: '',
    visited: new Set(),
    path: [],
    results: [],
    phase: 'idle',
    stepQueue: [],
    stepIndex: 0,
};
let canvas;
let ctx;
let W = 520;
let H = 480;
function toScreen(nx, ny) {
    const margin = 50;
    return {
        x: margin + nx * (W - 2 * margin),
        y: margin + ny * (H - 2 * margin),
    };
}
function resize() {
    const container = canvas.parentElement;
    const w = Math.min(container.clientWidth, 520);
    const h = Math.min(w * 0.92, 480);
    const dpr = window.devicePixelRatio || 1;
    W = w;
    H = h;
    canvas.width = w * dpr;
    canvas.height = h * dpr;
    canvas.style.width = w + 'px';
    canvas.style.height = h + 'px';
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    draw();
}
function draw() {
    ctx.fillStyle = COLORS.bg;
    ctx.fillRect(0, 0, W, H);
    // Determine visible layer
    const visLayer = state.phase === 'idle' ? -1 : state.currentLayer;
    // Draw edges
    for (const node of nodes.values()) {
        for (const [layer, neighbors] of node.neighbors) {
            if (visLayer >= 0 && layer > visLayer)
                continue;
            for (const nId of neighbors) {
                if (nId < node.id)
                    continue; // avoid double-draw
                const nb = nodes.get(nId);
                const p1 = toScreen(node.pos.x, node.pos.y);
                const p2 = toScreen(nb.pos.x, nb.pos.y);
                const isOnPath = state.path.some((s, i) => i > 0 && s.layer === layer &&
                    ((state.path[i - 1].nodeId === node.id && s.nodeId === nId) ||
                        (state.path[i - 1].nodeId === nId && s.nodeId === node.id)));
                ctx.strokeStyle = isOnPath ? LAYER_COLORS[layer] + 'cc' : COLORS.edge;
                ctx.lineWidth = isOnPath ? 2.5 : 0.8;
                if (layer > 0 && !isOnPath) {
                    ctx.setLineDash([4, 4]);
                }
                ctx.beginPath();
                ctx.moveTo(p1.x, p1.y);
                ctx.lineTo(p2.x, p2.y);
                ctx.stroke();
                ctx.setLineDash([]);
            }
        }
    }
    // Draw nodes
    for (const node of nodes.values()) {
        const p = toScreen(node.pos.x, node.pos.y);
        // Determine visual state
        const isCurrent = state.currentNode === node.id && state.phase === 'traversing';
        const isVisited = state.visited.has(node.id);
        const isResult = state.results.includes(node.id);
        const nodeLayer = node.layer;
        let color = LAYER_COLORS[Math.min(nodeLayer, 2)];
        let radius = 6 + nodeLayer * 2;
        if (isResult) {
            color = COLORS.query;
            radius = 10;
        }
        else if (isCurrent) {
            color = COLORS.current;
            radius = 10;
        }
        else if (isVisited) {
            color = COLORS.visited;
        }
        // Glow for current
        if (isCurrent) {
            ctx.beginPath();
            ctx.arc(p.x, p.y, radius + 6, 0, Math.PI * 2);
            ctx.fillStyle = color + '33';
            ctx.fill();
        }
        ctx.beginPath();
        ctx.arc(p.x, p.y, radius, 0, Math.PI * 2);
        ctx.fillStyle = color;
        ctx.fill();
        // Label
        ctx.fillStyle = COLORS.text;
        ctx.font = '10px "Segoe UI", system-ui, sans-serif';
        ctx.textAlign = 'center';
        ctx.textBaseline = 'bottom';
        ctx.fillText(node.name, p.x, p.y - radius - 3);
    }
    // Draw query point
    if (state.phase !== 'idle') {
        const qp = toScreen(1.0, 0.0);
        ctx.beginPath();
        ctx.arc(qp.x, qp.y, 8, 0, Math.PI * 2);
        ctx.fillStyle = COLORS.query;
        ctx.fill();
        ctx.fillStyle = COLORS.text;
        ctx.font = 'bold 10px "Segoe UI", system-ui, sans-serif';
        ctx.textAlign = 'center';
        ctx.textBaseline = 'bottom';
        ctx.fillText('Query: "John"', qp.x, qp.y - 12);
    }
    // Layer indicator
    if (state.phase === 'traversing') {
        ctx.fillStyle = LAYER_COLORS[Math.min(state.currentLayer, 2)];
        ctx.font = 'bold 12px "Segoe UI", system-ui, sans-serif';
        ctx.textAlign = 'left';
        ctx.textBaseline = 'top';
        ctx.fillText(`Layer ${state.currentLayer}`, 12, 12);
    }
}
function buildSearchSteps() {
    const steps = [];
    // Find entry point (highest layer node)
    const entryNode = 'alice1'; // known entry
    // Layer 2: greedy search
    steps.push(() => {
        state.currentLayer = 2;
        state.currentNode = entryNode;
        state.visited.add(entryNode);
        state.path.push({ nodeId: entryNode, layer: 2, dist: dist(nodes.get(entryNode).embedding, queryEmbedding) });
        updateStepsUI();
    });
    // Layer 2: check neighbors
    steps.push(() => {
        // bob1 is further, stay at alice1. Drop to layer 1.
        const bob = nodes.get('bob1');
        const alice = nodes.get('alice1');
        state.visited.add('bob1');
        // alice is closer to query than bob
        // Also check chen1
        state.visited.add('chen1');
        const chen = nodes.get('chen1');
        // chen1 is closer
        state.currentNode = 'chen1';
        state.path.push({ nodeId: 'chen1', layer: 2, dist: dist(chen.embedding, queryEmbedding) });
        updateStepsUI();
    });
    // Drop to layer 1
    steps.push(() => {
        state.currentLayer = 1;
        state.path.push({ nodeId: 'chen1', layer: 1, dist: dist(nodes.get('chen1').embedding, queryEmbedding) });
        updateStepsUI();
    });
    // Layer 1: check neighbors of chen1 -> johan1 is closer
    steps.push(() => {
        state.visited.add('johan1');
        state.visited.add('maria1');
        state.currentNode = 'johan1';
        state.path.push({ nodeId: 'johan1', layer: 1, dist: dist(nodes.get('johan1').embedding, queryEmbedding) });
        updateStepsUI();
    });
    // Drop to layer 0
    steps.push(() => {
        state.currentLayer = 0;
        state.path.push({ nodeId: 'johan1', layer: 0, dist: dist(nodes.get('johan1').embedding, queryEmbedding) });
        updateStepsUI();
    });
    // Layer 0: check neighbors -> john1 is closest
    steps.push(() => {
        state.visited.add('john1');
        state.currentNode = 'john1';
        state.path.push({ nodeId: 'john1', layer: 0, dist: dist(nodes.get('john1').embedding, queryEmbedding) });
        updateStepsUI();
    });
    // Expand from john1 -> find jean1, juan1
    steps.push(() => {
        state.visited.add('jean1');
        state.visited.add('juan1');
        // Collect top-K results (K=4)
        const candidates = ['john1', 'johan1', 'jean1', 'juan1'];
        state.results = candidates;
        state.phase = 'done';
        updateStepsUI();
        updateResultsUI();
    });
    return steps;
}
function updateStepsUI() {
    const container = document.getElementById('steps-container');
    container.innerHTML = '';
    for (let i = 0; i < state.path.length; i++) {
        const step = state.path[i];
        const node = nodes.get(step.nodeId);
        const isLast = i === state.path.length - 1 && state.phase !== 'done';
        const card = document.createElement('div');
        card.className = `step-card ${isLast ? 'active' : 'done'}`;
        card.innerHTML = `
      <div class="label">Layer ${step.layer} ${i > 0 && state.path[i - 1].layer !== step.layer ? '(dropped down)' : ''}</div>
      <div class="value">
        <span style="color:${LAYER_COLORS[Math.min(step.layer, 2)]}">${node.name}</span>
        &nbsp;&mdash;&nbsp; distance: <span class="highlight">${step.dist.toFixed(3)}</span>
      </div>
    `;
        container.appendChild(card);
    }
}
function updateResultsUI() {
    const container = document.getElementById('results-container');
    container.innerHTML = '';
    if (state.results.length === 0) {
        container.innerHTML = '<div class="explanation">Click "Search" to find vectors similar to the name "John"</div>';
        return;
    }
    for (const id of state.results) {
        const node = nodes.get(id);
        const d = dist(node.embedding, queryEmbedding);
        const card = document.createElement('div');
        card.className = 'result-card';
        card.innerHTML = `
      <div class="name">${node.name}</div>
      <div class="meta">Distance: ${d.toFixed(3)} &mdash; Similarity: ${(1 - d).toFixed(3)}</div>
      <div class="json-badge">JSON: { "fatherId": "${node.json.fatherId || 'null'}" }</div>
    `;
        container.appendChild(card);
    }
}
function resetState() {
    state = {
        running: false,
        currentLayer: 2,
        currentNode: '',
        visited: new Set(),
        path: [],
        results: [],
        phase: 'idle',
        stepQueue: [],
        stepIndex: 0,
    };
    document.getElementById('steps-container').innerHTML = '';
    updateResultsUI();
    draw();
}
function runSearch() {
    resetState();
    state.phase = 'traversing';
    state.stepQueue = buildSearchSteps();
    state.stepIndex = 0;
    // Auto-play
    state.running = true;
    playNext();
}
function playNext() {
    if (state.stepIndex >= state.stepQueue.length) {
        state.running = false;
        return;
    }
    state.stepQueue[state.stepIndex]();
    state.stepIndex++;
    draw();
    if (state.running && state.stepIndex < state.stepQueue.length) {
        setTimeout(playNext, 800);
    }
}
function stepOnce() {
    if (state.phase === 'idle') {
        state.phase = 'traversing';
        state.stepQueue = buildSearchSteps();
        state.stepIndex = 0;
    }
    state.running = false;
    if (state.stepIndex < state.stepQueue.length) {
        state.stepQueue[state.stepIndex]();
        state.stepIndex++;
        draw();
    }
}
// --- Init ---
document.addEventListener('DOMContentLoaded', () => {
    canvas = document.getElementById('canvas');
    ctx = canvas.getContext('2d');
    document.getElementById('btn-search').addEventListener('click', runSearch);
    document.getElementById('btn-reset').addEventListener('click', resetState);
    document.getElementById('btn-step').addEventListener('click', stepOnce);
    window.addEventListener('resize', resize);
    resize();
});
export {};
