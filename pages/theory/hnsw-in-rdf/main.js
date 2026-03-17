const COLORS = {
    bg: '#0a0a0f',
    edge: '#1e1e2a',
    nodeText: '#d0d0dc',
    vector: '#7c8cf8',
    graph: '#f87c7c',
    unified: '#34d399',
    hnsw: '#facc15',
    dimNode: '#2a2a3a',
    layer0: '#7c8cf8',
    layer1: '#facc15',
    layer2: '#f87c7c',
};
const nodes = new Map();
const triples = [];
function addNode(id, label, x, y, layer, hasEmb) {
    nodes.set(id, { id, label, pos: { x, y }, layer, hasEmbedding: hasEmb });
}
function addTriple(s, p, o, type) {
    triples.push({ s, p, o, type });
}
// Nodes
addNode('paper1', 'Paper A', 0.15, 0.20, 0, true);
addNode('paper2', 'Paper B', 0.35, 0.15, 1, true);
addNode('paper3', 'Paper C', 0.55, 0.20, 0, true);
addNode('paper4', 'Paper D', 0.75, 0.15, 2, true);
addNode('paper5', 'Paper E', 0.45, 0.40, 0, true);
addNode('paper6', 'Paper F', 0.25, 0.45, 1, true);
addNode('topic1', 'Transformers', 0.15, 0.70, -1, false);
addNode('topic2', 'Graph Neural Nets', 0.45, 0.70, -1, false);
addNode('topic3', 'HNSW', 0.75, 0.70, -1, false);
addNode('auth1', 'Alice', 0.30, 0.88, -1, false);
addNode('auth2', 'Bob', 0.60, 0.88, -1, false);
// Data triples (regular RDF)
addTriple('paper1', ':discusses', 'topic1', 'data');
addTriple('paper2', ':discusses', 'topic1', 'data');
addTriple('paper3', ':discusses', 'topic2', 'data');
addTriple('paper4', ':discusses', 'topic3', 'data');
addTriple('paper5', ':discusses', 'topic2', 'data');
addTriple('paper1', ':author', 'auth1', 'data');
addTriple('paper3', ':author', 'auth1', 'data');
addTriple('paper4', ':author', 'auth2', 'data');
// Vector triples (embedding stored as triple)
addTriple('paper1', ':hasEmbedding', '"0.23 -0.11 0.87..."^^sutra:f32vec', 'vector');
addTriple('paper2', ':hasEmbedding', '"0.25 -0.09 0.85..."^^sutra:f32vec', 'vector');
addTriple('paper3', ':hasEmbedding', '"0.44 0.31 0.22..."^^sutra:f32vec', 'vector');
addTriple('paper4', ':hasEmbedding', '"0.71 0.52 -0.33..."^^sutra:f32vec', 'vector');
// HNSW neighbor triples (virtual, generated on-the-fly)
addTriple('paper1', 'sutra:hnswNeighbor', 'paper2', 'hnsw');
addTriple('paper2', 'sutra:hnswNeighbor', 'paper3', 'hnsw');
addTriple('paper2', 'sutra:hnswNeighbor', 'paper5', 'hnsw');
addTriple('paper3', 'sutra:hnswNeighbor', 'paper5', 'hnsw');
addTriple('paper4', 'sutra:hnswNeighbor', 'paper3', 'hnsw');
addTriple('paper5', 'sutra:hnswNeighbor', 'paper6', 'hnsw');
addTriple('paper6', 'sutra:hnswNeighbor', 'paper1', 'hnsw');
let state = createInitialState();
function createInitialState() {
    return {
        phase: 'idle',
        running: false,
        visibleTypes: new Set(['data', 'vector', 'hnsw']),
        activeTriples: new Set(),
        activeNodes: new Set(),
        steps: [],
        stepQueue: [],
        stepIndex: 0,
    };
}
let canvas;
let ctx;
let W = 520;
let H = 480;
function toScreen(nx, ny) {
    const mx = 40, my = 35;
    return {
        x: mx + nx * (W - 2 * mx),
        y: my + ny * (H - 2 * my),
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
    const typeColors = {
        data: COLORS.graph,
        vector: COLORS.vector,
        hnsw: COLORS.hnsw,
    };
    // Draw edges for visible triple types
    for (let ti = 0; ti < triples.length; ti++) {
        const t = triples[ti];
        if (!state.visibleTypes.has(t.type))
            continue;
        const sNode = nodes.get(t.s);
        const oNode = nodes.get(t.o);
        if (!sNode || !oNode)
            continue;
        const from = toScreen(sNode.pos.x, sNode.pos.y);
        const to = toScreen(oNode.pos.x, oNode.pos.y);
        const isActive = state.activeTriples.has(ti);
        const color = typeColors[t.type];
        // HNSW edges are dashed
        if (t.type === 'hnsw') {
            ctx.setLineDash([4, 3]);
        }
        ctx.strokeStyle = isActive ? color : color + '44';
        ctx.lineWidth = isActive ? 2.5 : 0.8;
        ctx.beginPath();
        ctx.moveTo(from.x, from.y);
        ctx.lineTo(to.x, to.y);
        ctx.stroke();
        ctx.setLineDash([]);
        // Label on active edges
        if (isActive) {
            const mx = (from.x + to.x) / 2;
            const my = (from.y + to.y) / 2;
            ctx.fillStyle = color;
            ctx.font = '7px "Cascadia Code", monospace';
            ctx.textAlign = 'center';
            ctx.textBaseline = 'bottom';
            ctx.fillText(t.p, mx, my - 3);
        }
    }
    // Draw nodes
    for (const node of nodes.values()) {
        const p = toScreen(node.pos.x, node.pos.y);
        const isActive = state.activeNodes.has(node.id);
        let color = COLORS.dimNode;
        let radius = 7;
        if (isActive) {
            color = COLORS.unified;
            radius = 10;
        }
        else if (node.hasEmbedding) {
            color = node.layer >= 2 ? COLORS.layer2 : node.layer >= 1 ? COLORS.layer1 : COLORS.layer0;
            radius = 7 + node.layer * 2;
        }
        else {
            color = COLORS.graph;
            radius = 7;
        }
        if (state.phase !== 'idle' && !isActive && state.activeNodes.size > 0) {
            color = COLORS.dimNode;
            radius = 6;
        }
        if (isActive) {
            ctx.beginPath();
            ctx.arc(p.x, p.y, radius + 5, 0, Math.PI * 2);
            ctx.fillStyle = color + '33';
            ctx.fill();
        }
        ctx.beginPath();
        ctx.arc(p.x, p.y, radius, 0, Math.PI * 2);
        ctx.fillStyle = color;
        ctx.fill();
        // Embedding ring for vector nodes
        if (node.hasEmbedding && state.visibleTypes.has('vector')) {
            ctx.beginPath();
            ctx.arc(p.x, p.y, radius + 2, 0, Math.PI * 2);
            ctx.strokeStyle = COLORS.vector + '66';
            ctx.lineWidth = 1;
            ctx.stroke();
        }
        ctx.fillStyle = isActive ? '#ffffff' : COLORS.nodeText;
        ctx.font = `${isActive ? 'bold ' : ''}9px "Segoe UI", system-ui, sans-serif`;
        ctx.textAlign = 'center';
        ctx.textBaseline = 'bottom';
        ctx.fillText(node.label, p.x, p.y - radius - 2);
        // Layer badge for HNSW nodes
        if (node.layer >= 0 && node.hasEmbedding) {
            ctx.fillStyle = '#444458';
            ctx.font = '7px "Cascadia Code", monospace';
            ctx.textBaseline = 'top';
            ctx.fillText(`L${node.layer}`, p.x, p.y + radius + 1);
        }
    }
    // Legend
    ctx.font = '9px "Segoe UI", system-ui, sans-serif';
    ctx.textAlign = 'left';
    ctx.textBaseline = 'top';
    let ly = 8;
    const legendItems = [
        ['Data triples (:discusses, :author)', COLORS.graph, state.visibleTypes.has('data')],
        ['Vector triples (:hasEmbedding)', COLORS.vector, state.visibleTypes.has('vector')],
        ['HNSW triples (sutra:hnswNeighbor)', COLORS.hnsw, state.visibleTypes.has('hnsw')],
    ];
    for (const [label, col, vis] of legendItems) {
        ctx.fillStyle = vis ? col : col + '44';
        ctx.fillText(vis ? '\u25CF ' + label : '\u25CB ' + label, 8, ly);
        ly += 14;
    }
}
function buildSteps() {
    const steps = [];
    // Step 1: Show data triples only
    steps.push(() => {
        state.phase = 'data-triples';
        state.visibleTypes = new Set(['data']);
        // Highlight all data triples
        for (let i = 0; i < triples.length; i++) {
            if (triples[i].type === 'data')
                state.activeTriples.add(i);
        }
        state.steps.push({
            description: 'Layer 1: Regular RDF data triples',
            detail: ':discusses, :author \u2014 standard graph relationships stored in SPO/POS/OSP indexes. Nothing new here.',
            active: true, done: false,
        });
        updateStepsUI();
    });
    // Step 2: Add vector triples
    steps.push(() => {
        state.steps[0].active = false;
        state.steps[0].done = true;
        state.phase = 'vector-triples';
        state.visibleTypes = new Set(['data', 'vector']);
        state.activeTriples.clear();
        for (let i = 0; i < triples.length; i++) {
            if (triples[i].type === 'vector')
                state.activeTriples.add(i);
        }
        state.steps.push({
            description: 'Layer 2: Vectors are triples too',
            detail: ':hasEmbedding "0.23 -0.11 ..."^^sutra:f32vec \u2014 the embedding is a typed literal attached to the node via a regular predicate. Indexed by HNSW, but stored as a triple.',
            active: true, done: false,
        });
        updateStepsUI();
    });
    // Step 3: Add HNSW neighbor triples
    steps.push(() => {
        state.steps[1].active = false;
        state.steps[1].done = true;
        state.phase = 'hnsw-triples';
        state.visibleTypes = new Set(['data', 'vector', 'hnsw']);
        state.activeTriples.clear();
        for (let i = 0; i < triples.length; i++) {
            if (triples[i].type === 'hnsw')
                state.activeTriples.add(i);
        }
        state.steps.push({
            description: 'Layer 3: HNSW neighbors as virtual triples',
            detail: 'sutra:hnswNeighbor edges are generated on-the-fly from the HNSW index \u2014 not stored in SPO/POS/OSP. But they\'re queryable in SPARQL just like any other triple.',
            active: true, done: false,
        });
        updateStepsUI();
    });
    // Step 4: Unified query that uses all three
    steps.push(() => {
        state.steps[2].active = false;
        state.steps[2].done = true;
        state.phase = 'unified-query';
        state.activeTriples.clear();
        state.activeNodes.clear();
        // Simulate: find papers similar to Paper A, then get their topics
        state.activeNodes.add('paper1');
        state.activeNodes.add('paper2');
        state.activeNodes.add('paper6');
        state.activeNodes.add('topic1');
        // Highlight the HNSW edges used + data edges used
        for (let i = 0; i < triples.length; i++) {
            const t = triples[i];
            if (t.s === 'paper1' && t.p === 'sutra:hnswNeighbor' && t.o === 'paper2')
                state.activeTriples.add(i);
            if (t.s === 'paper6' && t.p === 'sutra:hnswNeighbor' && t.o === 'paper1')
                state.activeTriples.add(i);
            if (t.s === 'paper2' && t.p === ':discusses' && t.o === 'topic1')
                state.activeTriples.add(i);
            if (t.s === 'paper1' && t.p === ':discusses' && t.o === 'topic1')
                state.activeTriples.add(i);
        }
        state.steps.push({
            description: 'Unified query: HNSW + graph in one SPARQL walk',
            detail: 'Find papers similar to Paper A (via sutra:hnswNeighbor), then follow :discusses edges to get topics. One query, one graph, no JSON handoff.',
            active: true, done: false,
        });
        updateStepsUI();
    });
    // Step 5: Done
    steps.push(() => {
        state.steps[3].active = false;
        state.steps[3].done = true;
        state.phase = 'done';
        state.steps.push({
            description: 'Three layers, one graph, one query language',
            detail: 'Data triples, vector literals, and HNSW topology all live in the same RDF graph. SPARQL property paths can traverse HNSW edges just like :hasFather edges. The vector index is the 4th index alongside SPO/POS/OSP.',
            active: false, done: true,
        });
        updateStepsUI();
    });
    return steps;
}
function updateStepsUI() {
    const container = document.getElementById('steps-container');
    container.innerHTML = '';
    for (const step of state.steps) {
        const card = document.createElement('div');
        card.className = `step-card ${step.active ? 'active' : ''} ${step.done ? 'done' : ''}`;
        card.innerHTML = `
      <div class="label">${step.description}</div>
      <div class="value">${step.detail}</div>
    `;
        container.appendChild(card);
    }
}
function resetState() {
    state = createInitialState();
    document.getElementById('steps-container').innerHTML = '';
    draw();
}
function runDemo() {
    resetState();
    state.stepQueue = buildSteps();
    state.stepIndex = 0;
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
        setTimeout(playNext, 1200);
    }
}
function stepOnce() {
    if (state.phase === 'idle') {
        state.stepQueue = buildSteps();
        state.stepIndex = 0;
    }
    state.running = false;
    if (state.stepIndex < state.stepQueue.length) {
        state.stepQueue[state.stepIndex]();
        state.stepIndex++;
        draw();
    }
}
document.addEventListener('DOMContentLoaded', () => {
    canvas = document.getElementById('canvas');
    ctx = canvas.getContext('2d');
    document.getElementById('btn-run').addEventListener('click', runDemo);
    document.getElementById('btn-reset').addEventListener('click', resetState);
    document.getElementById('btn-step').addEventListener('click', stepOnce);
    window.addEventListener('resize', resize);
    resize();
});
export {};
