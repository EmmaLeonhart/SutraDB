const COLORS = {
    bg: '#0a0a0f',
    text: '#d0d0dc',
    textDim: '#707088',
    vector: '#7c8cf8',
    graph: '#f87c7c',
    unified: '#34d399',
    similarity: '#7c8cf844',
    fatherEdge: '#f87c7c',
    edge: '#1e1e2a',
    current: '#ffffff',
    nodeDim: '#2a2a3a',
};
const nodes = new Map();
function addNode(id, name, x, y, emb, fatherId) {
    nodes.set(id, { id, name, pos: { x, y }, embedding: emb, fatherId, vectorNeighbors: [] });
}
function addVectorEdge(a, b) {
    nodes.get(a).vectorNeighbors.push(b);
    nodes.get(b).vectorNeighbors.push(a);
}
// People with "John"-like names cluster together
addNode('john1', 'John Smith', 0.20, 0.08, [0.95, 0.05], 'robert1');
addNode('jean1', 'Jean Dupont', 0.14, 0.18, [0.88, 0.12], 'pierre1');
addNode('johan1', 'Johan Berg', 0.26, 0.14, [0.92, 0.08], 'erik1');
addNode('juan1', 'Juan Garcia', 0.10, 0.26, [0.85, 0.15], 'carlos1');
// Fathers
addNode('robert1', 'Robert Smith', 0.35, 0.28, [0.40, 0.55], 'william1');
addNode('pierre1', 'Pierre Dupont', 0.28, 0.40, [0.30, 0.60], 'louis1');
addNode('erik1', 'Erik Berg', 0.42, 0.35, [0.45, 0.50], 'olaf1');
addNode('carlos1', 'Carlos Garcia', 0.22, 0.48, [0.25, 0.58], 'miguel1');
// Grandfathers
addNode('william1', 'William Smith', 0.50, 0.52, [0.35, 0.75], 'henry1');
addNode('louis1', 'Louis Dupont', 0.38, 0.62, [0.20, 0.80], 'francois1');
addNode('olaf1', 'Olaf Berg', 0.58, 0.58, [0.50, 0.70], 'sven1');
addNode('miguel1', 'Miguel Garcia', 0.30, 0.70, [0.15, 0.75], 'antonio1');
// Great-grandfathers
addNode('henry1', 'Henry Smith', 0.55, 0.78, [0.40, 0.88]);
addNode('francois1', 'Francois Dupont', 0.40, 0.85, [0.18, 0.90]);
addNode('sven1', 'Sven Berg', 0.65, 0.82, [0.52, 0.85]);
addNode('antonio1', 'Antonio Garcia', 0.32, 0.90, [0.12, 0.92]);
// Other people (not John-like)
addNode('alice1', 'Alice Johnson', 0.75, 0.25, [0.50, 0.45]);
addNode('chen1', 'Chen Wei', 0.80, 0.45, [0.80, 0.55]);
// Vector similarity edges (HNSW connections)
addVectorEdge('john1', 'jean1');
addVectorEdge('john1', 'johan1');
addVectorEdge('john1', 'juan1');
addVectorEdge('jean1', 'juan1');
addVectorEdge('johan1', 'alice1');
addVectorEdge('alice1', 'chen1');
addVectorEdge('chen1', 'johan1');
function dist(a, b) {
    const dx = a[0] - b[0];
    const dy = a[1] - b[1];
    return Math.sqrt(dx * dx + dy * dy);
}
let state = createInitialState();
function createInitialState() {
    return {
        phase: 'idle',
        running: false,
        currentNode: '',
        vectorCursor: '',
        visitedVector: new Set(),
        visitedGraph: new Set(),
        activeEdges: new Set(),
        fatherEdges: new Set(),
        results: [],
        steps: [],
        stepQueue: [],
        stepIndex: 0,
        operations: 0,
    };
}
let canvas;
let ctx;
let W = 520;
let H = 520;
function toScreen(nx, ny) {
    const mx = 50, my = 40;
    return {
        x: mx + nx * (W - 2 * mx),
        y: my + ny * (H - 2 * my),
    };
}
function resize() {
    const container = canvas.parentElement;
    const w = Math.min(container.clientWidth, 520);
    const h = Math.min(w, 520);
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
    // Draw vector similarity edges (dashed blue)
    for (const node of nodes.values()) {
        for (const nId of node.vectorNeighbors) {
            if (nId < node.id)
                continue;
            const nb = nodes.get(nId);
            const from = toScreen(node.pos.x, node.pos.y);
            const to = toScreen(nb.pos.x, nb.pos.y);
            const edgeKey = `v:${node.id}->${nId}`;
            const isActive = state.activeEdges.has(edgeKey) || state.activeEdges.has(`v:${nId}->${node.id}`);
            ctx.strokeStyle = isActive ? COLORS.vector : COLORS.similarity;
            ctx.lineWidth = isActive ? 2 : 0.6;
            ctx.setLineDash([4, 3]);
            ctx.beginPath();
            ctx.moveTo(from.x, from.y);
            ctx.lineTo(to.x, to.y);
            ctx.stroke();
            ctx.setLineDash([]);
        }
    }
    // Draw :hasFather edges (solid red)
    for (const node of nodes.values()) {
        if (!node.fatherId)
            continue;
        const father = nodes.get(node.fatherId);
        if (!father)
            continue;
        const from = toScreen(node.pos.x, node.pos.y);
        const to = toScreen(father.pos.x, father.pos.y);
        const edgeKey = `g:${node.id}->${node.fatherId}`;
        const isTraversed = state.fatherEdges.has(edgeKey);
        ctx.strokeStyle = isTraversed ? COLORS.fatherEdge : COLORS.edge;
        ctx.lineWidth = isTraversed ? 2.5 : 0.8;
        ctx.beginPath();
        ctx.moveTo(from.x, from.y);
        ctx.lineTo(to.x, to.y);
        ctx.stroke();
        // Arrowhead
        if (isTraversed) {
            const dx = to.x - from.x;
            const dy = to.y - from.y;
            const len = Math.sqrt(dx * dx + dy * dy);
            const nx = dx / len;
            const ny = dy / len;
            const ex = to.x - nx * 12;
            const ey = to.y - ny * 12;
            ctx.fillStyle = COLORS.fatherEdge;
            ctx.beginPath();
            ctx.moveTo(ex + ny * 4, ey - nx * 4);
            ctx.lineTo(to.x - nx * 8, to.y - ny * 8);
            ctx.lineTo(ex - ny * 4, ey + nx * 4);
            ctx.closePath();
            ctx.fill();
        }
        // Small label
        if (isTraversed) {
            const mx = (from.x + to.x) / 2;
            const my = (from.y + to.y) / 2;
            ctx.fillStyle = COLORS.fatherEdge;
            ctx.font = '8px "Cascadia Code", monospace';
            ctx.textAlign = 'center';
            ctx.textBaseline = 'bottom';
            ctx.fillText(':hasFather', mx + 5, my - 3);
        }
    }
    // Draw nodes
    for (const node of nodes.values()) {
        const p = toScreen(node.pos.x, node.pos.y);
        const isVectorVisited = state.visitedVector.has(node.id);
        const isGraphVisited = state.visitedGraph.has(node.id);
        const isCurrent = state.currentNode === node.id;
        const isVectorCursor = state.vectorCursor === node.id;
        const isResult = state.results.some(r => r.greatgrandfather === node.name);
        let color = COLORS.vector;
        let radius = 7;
        if (isResult) {
            color = COLORS.unified;
            radius = 10;
        }
        else if (isCurrent) {
            color = COLORS.current;
            radius = 10;
        }
        else if (isVectorCursor) {
            color = COLORS.vector;
            radius = 10;
        }
        else if (isVectorVisited && isGraphVisited) {
            color = COLORS.unified;
            radius = 9;
        }
        else if (isVectorVisited) {
            color = COLORS.vector;
            radius = 9;
        }
        else if (isGraphVisited) {
            color = COLORS.fatherEdge;
            radius = 8;
        }
        else if (state.phase !== 'idle') {
            color = COLORS.nodeDim;
            radius = 5;
        }
        // Glow
        if (isCurrent || isResult || isVectorCursor) {
            ctx.beginPath();
            ctx.arc(p.x, p.y, radius + 6, 0, Math.PI * 2);
            ctx.fillStyle = color + '33';
            ctx.fill();
        }
        // Dual-ring for unified nodes (visited by both vector + graph)
        if (isVectorVisited && isGraphVisited && !isCurrent && !isResult) {
            ctx.beginPath();
            ctx.arc(p.x, p.y, radius + 2, 0, Math.PI * 2);
            ctx.strokeStyle = COLORS.vector + '66';
            ctx.lineWidth = 1.5;
            ctx.stroke();
        }
        ctx.beginPath();
        ctx.arc(p.x, p.y, radius, 0, Math.PI * 2);
        ctx.fillStyle = color;
        ctx.fill();
        // Label
        ctx.fillStyle = (isCurrent || isResult || isVectorVisited || isGraphVisited || isVectorCursor) ? COLORS.text : COLORS.textDim;
        ctx.font = `${isCurrent || isResult ? 'bold ' : ''}9px "Segoe UI", system-ui, sans-serif`;
        ctx.textAlign = 'center';
        ctx.textBaseline = 'bottom';
        ctx.fillText(node.name, p.x, p.y - radius - 3);
    }
    // Query indicator
    if (state.phase !== 'idle') {
        ctx.fillStyle = COLORS.unified;
        ctx.font = 'bold 11px "Segoe UI", system-ui, sans-serif';
        ctx.textAlign = 'left';
        ctx.textBaseline = 'top';
        ctx.fillText('sutraDB: "John" + :hasFather x3', 8, 8);
        // Legend
        ctx.font = '9px "Segoe UI", system-ui, sans-serif';
        ctx.fillStyle = COLORS.vector;
        ctx.fillText('--- vector similarity', 8, 24);
        ctx.fillStyle = COLORS.fatherEdge;
        ctx.fillText('— :hasFather', 140, 24);
        // Show async indicator when both are happening
        const hasActiveVector = state.steps.some(s => s.type === 'vector' && s.active);
        const hasActiveGraph = state.steps.some(s => s.type === 'graph' && s.active);
        const hasActiveBoth = state.steps.some(s => s.type === 'both' && s.active);
        if (hasActiveBoth || (hasActiveVector && hasActiveGraph)) {
            ctx.fillStyle = COLORS.unified;
            ctx.font = 'bold 9px "Cascadia Code", monospace';
            ctx.textAlign = 'right';
            ctx.fillText('vector + graph running concurrently', W - 8, 8);
        }
    }
}
// The key change: steps interleave vector discovery with graph traversal.
// Each time the vector search lands on a similar node, graph traversal
// kicks off immediately for that node — it doesn't wait for all vector
// results to come back first.
function buildSteps() {
    const steps = [];
    // Step 1: HNSW entry — start at Alice (far from query, chaotic start)
    steps.push(() => {
        state.visitedVector.add('alice1');
        state.vectorCursor = 'alice1';
        state.currentNode = 'alice1';
        state.operations += 1;
        state.steps.push({
            description: 'HNSW entry: Alice Johnson (random start)',
            detail: 'Vector traversal enters at Alice — far from "John." Greedy search begins...',
            type: 'vector',
            active: true,
            done: false,
        });
        updateStepsUI();
    });
    // Step 2: Hop to Chen Wei (closer)
    steps.push(() => {
        state.steps[0].active = false;
        state.steps[0].done = true;
        state.visitedVector.add('chen1');
        state.activeEdges.add('v:alice1->chen1');
        state.vectorCursor = 'chen1';
        state.currentNode = 'chen1';
        state.operations += 1;
        state.steps.push({
            description: 'HNSW hop → Chen Wei (getting closer)',
            detail: 'Greedy hop through the vector graph. No :hasFather here, keep going...',
            type: 'vector',
            active: true,
            done: false,
        });
        updateStepsUI();
    });
    // Step 3: Hop to Johan Berg — HAS a father! Immediately follow
    steps.push(() => {
        state.steps[1].active = false;
        state.steps[1].done = true;
        state.visitedVector.add('johan1');
        state.activeEdges.add('v:chen1->johan1');
        state.vectorCursor = 'johan1';
        state.currentNode = 'johan1';
        state.operations += 1;
        state.steps.push({
            description: 'HNSW hop → Johan Berg (sim 0.93) — has :hasFather!',
            detail: 'Landed on a similar name AND it has :hasFather edges. Follow them NOW, don\'t wait...',
            type: 'vector',
            active: true,
            done: false,
        });
        updateStepsUI();
    });
    // Step 4: Immediately follow Johan's father chain (vector search hasn't finished!)
    // state.steps indices: [0]=Alice, [1]=Chen, [2]=Johan. Now adding [3]=Johan chain
    steps.push(() => {
        state.steps[2].active = false;
        state.steps[2].done = true;
        state.visitedGraph.add('johan1');
        state.visitedGraph.add('erik1');
        state.fatherEdges.add('g:johan1->erik1');
        state.visitedGraph.add('olaf1');
        state.fatherEdges.add('g:erik1->olaf1');
        state.visitedGraph.add('sven1');
        state.fatherEdges.add('g:olaf1->sven1');
        state.currentNode = 'sven1';
        state.operations += 1;
        state.results.push({
            person: 'Johan Berg', greatgrandfather: 'Sven Berg',
            chain: 'Johan → Erik → Olaf → Sven', similarity: 0.93,
        });
        state.steps.push({
            description: 'Johan\'s :hasFather chain (3 hops, same graph!)',
            detail: 'Johan → Erik → Olaf → Sven. First result in! Vector search continues...',
            type: 'both',
            active: true,
            done: false,
        });
        updateStepsUI();
        updateResultsUI();
    });
    // Step 5: Vector search continues from Johan's neighborhood → John Smith
    // state.steps: [0,1,2,3]. Adding [4]
    steps.push(() => {
        state.steps[3].active = false;
        state.steps[3].done = true;
        state.visitedVector.add('john1');
        state.activeEdges.add('v:johan1->john1');
        state.vectorCursor = 'john1';
        state.currentNode = 'john1';
        state.operations += 1;
        state.steps.push({
            description: 'HNSW continues → John Smith (sim 0.98)',
            detail: 'Hopped from Johan to John via vector edge. Has :hasFather — follow immediately...',
            type: 'vector',
            active: true,
            done: false,
        });
        updateStepsUI();
    });
    // Step 6: Immediately follow John's chain
    // state.steps: [0..4]. Adding [5]
    steps.push(() => {
        state.steps[4].active = false;
        state.steps[4].done = true;
        state.visitedGraph.add('john1');
        state.visitedGraph.add('robert1');
        state.fatherEdges.add('g:john1->robert1');
        state.visitedGraph.add('william1');
        state.fatherEdges.add('g:robert1->william1');
        state.visitedGraph.add('henry1');
        state.fatherEdges.add('g:william1->henry1');
        state.currentNode = 'henry1';
        state.operations += 1;
        state.results.push({
            person: 'John Smith', greatgrandfather: 'Henry Smith',
            chain: 'John → Robert → William → Henry', similarity: 0.98,
        });
        state.steps.push({
            description: 'John\'s :hasFather chain (3 hops, same graph)',
            detail: 'John → Robert → William → Henry. Vector search still going...',
            type: 'both',
            active: true,
            done: false,
        });
        updateStepsUI();
        updateResultsUI();
    });
    // Step 7: Vector search expands John's neighbors → Jean Dupont
    // state.steps: [0..5]. Adding [6]
    steps.push(() => {
        state.steps[5].active = false;
        state.steps[5].done = true;
        state.visitedVector.add('jean1');
        state.activeEdges.add('v:john1->jean1');
        state.vectorCursor = 'jean1';
        state.currentNode = 'jean1';
        state.operations += 1;
        state.steps.push({
            description: 'HNSW expands → Jean Dupont (sim 0.89)',
            detail: 'Vector neighbor of John. Has :hasFather — follow immediately...',
            type: 'vector',
            active: true,
            done: false,
        });
        updateStepsUI();
    });
    // Step 8: Immediately follow Jean's chain
    // state.steps: [0..6]. Adding [7]
    steps.push(() => {
        state.steps[6].active = false;
        state.steps[6].done = true;
        state.visitedGraph.add('jean1');
        state.visitedGraph.add('pierre1');
        state.fatherEdges.add('g:jean1->pierre1');
        state.visitedGraph.add('louis1');
        state.fatherEdges.add('g:pierre1->louis1');
        state.visitedGraph.add('francois1');
        state.fatherEdges.add('g:louis1->francois1');
        state.currentNode = 'francois1';
        state.operations += 1;
        state.results.push({
            person: 'Jean Dupont', greatgrandfather: 'Francois Dupont',
            chain: 'Jean → Pierre → Louis → Francois', similarity: 0.89,
        });
        state.steps.push({
            description: 'Jean\'s :hasFather chain (3 hops, same graph)',
            detail: 'Jean → Pierre → Louis → Francois. Vector search still going...',
            type: 'both',
            active: true,
            done: false,
        });
        updateStepsUI();
        updateResultsUI();
    });
    // Step 9: Vector search continues → Juan Garcia
    // state.steps: [0..7]. Adding [8]
    steps.push(() => {
        state.steps[7].active = false;
        state.steps[7].done = true;
        state.visitedVector.add('juan1');
        state.activeEdges.add('v:john1->juan1');
        state.vectorCursor = 'juan1';
        state.currentNode = 'juan1';
        state.operations += 1;
        state.steps.push({
            description: 'HNSW expands → Juan Garcia (sim 0.86)',
            detail: 'Another vector neighbor. Has :hasFather — follow immediately...',
            type: 'vector',
            active: true,
            done: false,
        });
        updateStepsUI();
    });
    // Step 10: Immediately follow Juan's chain — done!
    // state.steps: [0..8]. Adding [9]
    steps.push(() => {
        state.steps[8].active = false;
        state.steps[8].done = true;
        state.visitedGraph.add('juan1');
        state.visitedGraph.add('carlos1');
        state.fatherEdges.add('g:juan1->carlos1');
        state.visitedGraph.add('miguel1');
        state.fatherEdges.add('g:carlos1->miguel1');
        state.visitedGraph.add('antonio1');
        state.fatherEdges.add('g:miguel1->antonio1');
        state.currentNode = '';
        state.vectorCursor = '';
        state.operations += 1;
        state.results.push({
            person: 'Juan Garcia', greatgrandfather: 'Antonio Garcia',
            chain: 'Juan → Carlos → Miguel → Antonio', similarity: 0.86,
        });
        state.steps.push({
            description: 'Juan\'s :hasFather chain → All results in!',
            detail: 'Juan → Carlos → Miguel → Antonio. 4 great-grandfathers found via interleaved traversal.',
            type: 'both',
            active: false,
            done: true,
        });
        state.phase = 'done';
        updateStepsUI();
        updateResultsUI();
        updateComparisonUI();
    });
    return steps;
}
function updateStepsUI() {
    const container = document.getElementById('steps-container');
    container.innerHTML = '';
    for (const step of state.steps) {
        const card = document.createElement('div');
        card.className = `step-card ${step.active ? 'active' : ''} ${step.done ? 'done' : ''}`;
        const typeColors = { vector: '#7c8cf8', graph: '#f87c7c', both: '#34d399' };
        const typeLabels = { vector: 'vector', graph: 'graph', both: 'vector + graph' };
        const typeBadge = `<span style="display:inline-block;font-size:0.65rem;background:${typeColors[step.type]}22;color:${typeColors[step.type]};padding:1px 5px;border-radius:3px;margin-left:4px;">${typeLabels[step.type]}</span>`;
        card.innerHTML = `
      <div class="label">${step.description} ${typeBadge}</div>
      <div class="value">${step.detail}</div>
    `;
        container.appendChild(card);
    }
}
function updateResultsUI() {
    const container = document.getElementById('results-container');
    container.innerHTML = '';
    if (state.results.length === 0) {
        container.innerHTML = '<div class="explanation">Click "Run Unified Query" to find great-grandfathers of people named like "John"</div>';
        return;
    }
    for (const r of state.results) {
        const card = document.createElement('div');
        card.className = 'result-card';
        card.innerHTML = `
      <div class="name">${r.person} → <span class="highlight-green">${r.greatgrandfather}</span></div>
      <div class="chain">${r.chain} (similarity: ${r.similarity.toFixed(2)})</div>
    `;
        container.appendChild(card);
    }
}
function updateComparisonUI() {
    document.getElementById('trad-ops').textContent = '21';
    document.getElementById('sutra-ops').textContent = String(state.operations);
    document.getElementById('trad-boundaries').textContent = '2';
    document.getElementById('sutra-boundaries').textContent = '0';
}
function resetState() {
    state = createInitialState();
    document.getElementById('steps-container').innerHTML = '';
    document.getElementById('results-container').innerHTML =
        '<div class="explanation">Click "Run Unified Query" to find great-grandfathers of people named like "John"</div>';
    document.getElementById('trad-ops').textContent = '—';
    document.getElementById('sutra-ops').textContent = '—';
    document.getElementById('trad-boundaries').textContent = '—';
    draw();
}
function runQuery() {
    resetState();
    state.phase = 'running';
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
        setTimeout(playNext, 800);
    }
}
function stepOnce() {
    if (state.phase === 'idle') {
        state.phase = 'running';
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
// --- Init ---
document.addEventListener('DOMContentLoaded', () => {
    canvas = document.getElementById('canvas');
    ctx = canvas.getContext('2d');
    document.getElementById('btn-run').addEventListener('click', runQuery);
    document.getElementById('btn-reset').addEventListener('click', resetState);
    document.getElementById('btn-step').addEventListener('click', stepOnce);
    window.addEventListener('resize', resize);
    resize();
});
export {};
