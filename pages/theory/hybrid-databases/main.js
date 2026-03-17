const COLORS = {
    bg: '#0a0a0f',
    text: '#d0d0dc',
    textDim: '#707088',
    vector: '#7c8cf8',
    graph: '#f87c7c',
    json: '#facc15',
    success: '#34d399',
    edge: '#1e1e2a',
    boundary: '#facc1544',
    nodeDim: '#2a2a3a',
};
// Vector space (left half) — includes HNSW neighbor connections to show the chaotic traversal
const vectorNodes = [
    { id: 'v1', name: 'John Smith', pos: { x: 0.35, y: 0.15 }, similarity: 0.98, json: { graphId: 'g-john1', fatherId: 'g-robert1' }, hnswNeighbors: ['v2', 'v3', 'v4'] },
    { id: 'v2', name: 'Jean Dupont', pos: { x: 0.30, y: 0.25 }, similarity: 0.89, json: { graphId: 'g-jean1', fatherId: 'g-pierre1' }, hnswNeighbors: ['v1', 'v4'] },
    { id: 'v3', name: 'Johan Berg', pos: { x: 0.38, y: 0.10 }, similarity: 0.93, json: { graphId: 'g-johan1', fatherId: 'g-erik1' }, hnswNeighbors: ['v1', 'v8'] },
    { id: 'v4', name: 'Juan Garcia', pos: { x: 0.25, y: 0.30 }, similarity: 0.86, json: { graphId: 'g-juan1', fatherId: 'g-carlos1' }, hnswNeighbors: ['v1', 'v2'] },
    { id: 'v5', name: 'Alice Johnson', pos: { x: 0.20, y: 0.60 }, similarity: 0.12, json: { graphId: 'g-alice1' }, hnswNeighbors: ['v6', 'v7', 'v8'] },
    { id: 'v6', name: 'Bob Williams', pos: { x: 0.15, y: 0.75 }, similarity: 0.08, json: { graphId: 'g-bob1' }, hnswNeighbors: ['v5'] },
    { id: 'v7', name: 'Maria Rodriguez', pos: { x: 0.30, y: 0.50 }, similarity: 0.22, json: { graphId: 'g-maria1' }, hnswNeighbors: ['v5', 'v8'] },
    { id: 'v8', name: 'Chen Wei', pos: { x: 0.40, y: 0.40 }, similarity: 0.35, json: { graphId: 'g-chen1' }, hnswNeighbors: ['v3', 'v5', 'v7'] },
];
// Graph space (right half) - family trees
const graphPeople = new Map();
function addG(id, name, x, y, fatherId) {
    graphPeople.set(id, { id, name, pos: { x, y }, fatherId });
}
addG('g-john1', 'John Smith', 0.60, 0.10, 'g-robert1');
addG('g-robert1', 'Robert Smith', 0.60, 0.30, 'g-william1');
addG('g-william1', 'William Smith', 0.60, 0.50, 'g-henry1');
addG('g-henry1', 'Henry Smith', 0.60, 0.70);
addG('g-jean1', 'Jean Dupont', 0.75, 0.10, 'g-pierre1');
addG('g-pierre1', 'Pierre Dupont', 0.75, 0.30, 'g-louis1');
addG('g-louis1', 'Louis Dupont', 0.75, 0.50, 'g-francois1');
addG('g-francois1', 'Francois Dupont', 0.75, 0.70);
addG('g-johan1', 'Johan Berg', 0.90, 0.10, 'g-erik1');
addG('g-erik1', 'Erik Berg', 0.90, 0.30, 'g-olaf1');
addG('g-olaf1', 'Olaf Berg', 0.90, 0.50, 'g-sven1');
addG('g-sven1', 'Sven Berg', 0.90, 0.70);
addG('g-juan1', 'Juan Garcia', 0.60, 0.85, 'g-carlos1');
addG('g-carlos1', 'Carlos Garcia', 0.75, 0.85, 'g-miguel1');
addG('g-miguel1', 'Miguel Garcia', 0.90, 0.85, 'g-antonio1');
addG('g-antonio1', 'Antonio Garcia', 0.90, 0.95);
let state = createInitialState();
function createInitialState() {
    return {
        phase: 'idle',
        running: false,
        currentPhaseIndex: -1,
        phases: [],
        activeVectorNodes: new Set(),
        vectorCursor: '',
        visitedVectorNodes: new Set(),
        hnswTraversedEdges: new Set(),
        jsonHighlighted: new Set(),
        activeGraphNodes: new Set(),
        traversedGraphEdges: new Set(),
        results: [],
        stepQueue: [],
        stepIndex: 0,
        operations: 0,
        systemBoundaries: 0,
    };
}
let canvas;
let ctx;
let W = 520;
let H = 480;
function toScreen(nx, ny) {
    const mx = 30, my = 30;
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
    // Draw boundary line
    const midX = W * 0.48;
    ctx.strokeStyle = COLORS.boundary;
    ctx.lineWidth = 1;
    ctx.setLineDash([6, 4]);
    ctx.beginPath();
    ctx.moveTo(midX, 10);
    ctx.lineTo(midX, H - 10);
    ctx.stroke();
    ctx.setLineDash([]);
    // Labels
    ctx.font = 'bold 11px "Segoe UI", system-ui, sans-serif';
    ctx.textAlign = 'center';
    ctx.fillStyle = COLORS.vector;
    ctx.fillText('Vector Database (HNSW)', midX * 0.5, 20);
    ctx.fillStyle = COLORS.graph;
    ctx.fillText('Graph Database (RDF)', midX + (W - midX) * 0.5, 20);
    // Draw HNSW edges in vector space
    for (const vn of vectorNodes) {
        for (const nId of vn.hnswNeighbors) {
            if (nId < vn.id)
                continue; // avoid double-draw
            const nb = vectorNodes.find(v => v.id === nId);
            if (!nb)
                continue;
            const from = toScreen(vn.pos.x, vn.pos.y);
            const to = toScreen(nb.pos.x, nb.pos.y);
            const edgeKey = `${vn.id}->${nId}`;
            const edgeKeyRev = `${nId}->${vn.id}`;
            const isTraversed = state.hnswTraversedEdges.has(edgeKey) || state.hnswTraversedEdges.has(edgeKeyRev);
            ctx.strokeStyle = isTraversed ? COLORS.vector + 'cc' : '#1a1a28';
            ctx.lineWidth = isTraversed ? 2 : 0.5;
            ctx.setLineDash([3, 3]);
            ctx.beginPath();
            ctx.moveTo(from.x, from.y);
            ctx.lineTo(to.x, to.y);
            ctx.stroke();
            ctx.setLineDash([]);
        }
    }
    // Draw vector nodes
    for (const vn of vectorNodes) {
        const p = toScreen(vn.pos.x, vn.pos.y);
        const isActive = state.activeVectorNodes.has(vn.id);
        const isCursor = state.vectorCursor === vn.id;
        const isVisited = state.visitedVectorNodes.has(vn.id);
        const isJsonHighlighted = state.jsonHighlighted.has(vn.id);
        let color = COLORS.vector;
        let radius = 6;
        if (isJsonHighlighted) {
            color = COLORS.json;
            radius = 8;
        }
        else if (isCursor) {
            color = '#ffffff';
            radius = 9;
        }
        else if (isActive) {
            color = COLORS.vector;
            radius = 8;
        }
        else if (isVisited) {
            color = COLORS.vector + 'aa';
            radius = 6;
        }
        else if (state.phase !== 'idle' && !isActive) {
            color = COLORS.nodeDim;
        }
        if (isActive || isJsonHighlighted || isCursor) {
            ctx.beginPath();
            ctx.arc(p.x, p.y, radius + 5, 0, Math.PI * 2);
            ctx.fillStyle = (isCursor ? '#ffffff' : color) + '33';
            ctx.fill();
        }
        ctx.beginPath();
        ctx.arc(p.x, p.y, radius, 0, Math.PI * 2);
        ctx.fillStyle = color;
        ctx.fill();
        ctx.fillStyle = isActive || isJsonHighlighted || isCursor ? '#ffffff' : COLORS.textDim;
        ctx.font = '9px "Segoe UI", system-ui, sans-serif';
        ctx.textAlign = 'center';
        ctx.textBaseline = 'bottom';
        ctx.fillText(vn.name, p.x, p.y - radius - 2);
    }
    // Draw graph edges
    for (const gp of graphPeople.values()) {
        if (!gp.fatherId)
            continue;
        const father = graphPeople.get(gp.fatherId);
        if (!father)
            continue;
        const from = toScreen(gp.pos.x, gp.pos.y);
        const to = toScreen(father.pos.x, father.pos.y);
        const edgeKey = `${gp.id}->${gp.fatherId}`;
        const isTraversed = state.traversedGraphEdges.has(edgeKey);
        ctx.strokeStyle = isTraversed ? COLORS.graph : COLORS.edge;
        ctx.lineWidth = isTraversed ? 2 : 0.8;
        ctx.beginPath();
        ctx.moveTo(from.x, from.y);
        ctx.lineTo(to.x, to.y);
        ctx.stroke();
        // Arrow head
        if (isTraversed) {
            const dx = to.x - from.x;
            const dy = to.y - from.y;
            const len = Math.sqrt(dx * dx + dy * dy);
            const nx = dx / len;
            const ny = dy / len;
            const ax = to.x - nx * 10;
            const ay = to.y - ny * 10;
            ctx.fillStyle = COLORS.graph;
            ctx.beginPath();
            ctx.moveTo(ax + ny * 4, ay - nx * 4);
            ctx.lineTo(to.x - nx * 6, to.y - ny * 6);
            ctx.lineTo(ax - ny * 4, ay + nx * 4);
            ctx.closePath();
            ctx.fill();
        }
    }
    // Draw graph nodes
    for (const gp of graphPeople.values()) {
        const p = toScreen(gp.pos.x, gp.pos.y);
        const isActive = state.activeGraphNodes.has(gp.id);
        let color = COLORS.graph;
        let radius = 6;
        if (isActive) {
            radius = 8;
            ctx.beginPath();
            ctx.arc(p.x, p.y, radius + 5, 0, Math.PI * 2);
            ctx.fillStyle = color + '33';
            ctx.fill();
        }
        else if (state.phase !== 'idle' && !isActive && state.activeGraphNodes.size > 0) {
            color = COLORS.nodeDim;
        }
        ctx.beginPath();
        ctx.arc(p.x, p.y, radius, 0, Math.PI * 2);
        ctx.fillStyle = color;
        ctx.fill();
        ctx.fillStyle = isActive ? '#ffffff' : COLORS.textDim;
        ctx.font = '9px "Segoe UI", system-ui, sans-serif';
        ctx.textAlign = 'center';
        ctx.textBaseline = 'bottom';
        ctx.fillText(gp.name, p.x, p.y - radius - 2);
    }
    // Draw JSON transfer arrows when active
    for (const vId of state.jsonHighlighted) {
        const vn = vectorNodes.find(v => v.id === vId);
        if (!vn)
            continue;
        const gp = graphPeople.get(vn.json.graphId);
        if (!gp)
            continue;
        const from = toScreen(vn.pos.x, vn.pos.y);
        const to = toScreen(gp.pos.x, gp.pos.y);
        ctx.strokeStyle = COLORS.json + '88';
        ctx.lineWidth = 1.5;
        ctx.setLineDash([3, 3]);
        ctx.beginPath();
        ctx.moveTo(from.x, from.y);
        ctx.lineTo(to.x, to.y);
        ctx.stroke();
        ctx.setLineDash([]);
        // "JSON" label
        const mx = (from.x + to.x) / 2;
        const my = (from.y + to.y) / 2;
        ctx.fillStyle = COLORS.json;
        ctx.font = 'bold 8px "Cascadia Code", monospace';
        ctx.textAlign = 'center';
        ctx.textBaseline = 'middle';
        ctx.fillText('JSON', mx, my - 8);
    }
}
function buildSteps() {
    const steps = [];
    // Phase 1a: HNSW entry — start at Alice (random entry point, far from query)
    steps.push(() => {
        state.phases.push({
            title: 'Phase 1: HNSW Traversal (Vector Search)',
            detail: 'Enter at Alice Johnson — random entry point, far from "John"',
            status: 'active',
        });
        state.vectorCursor = 'v5';
        state.visitedVectorNodes.add('v5');
        state.operations += 1;
        updatePhasesUI();
    });
    // Phase 1b: Hop to Chen Wei (greedy — closer to query)
    steps.push(() => {
        state.phases[0].detail = 'Hop → Chen Wei (closer to "John" than Alice)';
        state.hnswTraversedEdges.add('v5->v8');
        state.vectorCursor = 'v8';
        state.visitedVectorNodes.add('v8');
        state.operations += 1;
        updatePhasesUI();
    });
    // Phase 1c: Hop to Johan Berg (getting closer)
    steps.push(() => {
        state.phases[0].detail = 'Hop → Johan Berg (similarity 0.93 — getting warm)';
        state.hnswTraversedEdges.add('v8->v3');
        state.vectorCursor = 'v3';
        state.visitedVectorNodes.add('v3');
        state.activeVectorNodes.add('v3');
        state.operations += 1;
        updatePhasesUI();
    });
    // Phase 1d: Hop to John Smith — expand neighbors, collect results
    steps.push(() => {
        state.phases[0].detail = 'Hop → John Smith (0.98)! Expand neighbors: Jean, Juan';
        state.hnswTraversedEdges.add('v3->v1');
        state.hnswTraversedEdges.add('v1->v2');
        state.hnswTraversedEdges.add('v1->v4');
        state.vectorCursor = 'v1';
        state.visitedVectorNodes.add('v1');
        state.visitedVectorNodes.add('v2');
        state.visitedVectorNodes.add('v4');
        state.activeVectorNodes.add('v1');
        state.activeVectorNodes.add('v2');
        state.activeVectorNodes.add('v4');
        state.operations += 1;
        updatePhasesUI();
    });
    // Phase 1e: HNSW done — must now WAIT for all results before proceeding
    steps.push(() => {
        state.phases[0].status = 'done';
        state.phases[0].detail = 'Done! Found 4 results. Now must parse JSON and switch to graph DB...';
        state.vectorCursor = '';
        updatePhasesUI();
    });
    // Phase 2: Parse JSON metadata
    steps.push(() => {
        state.phases.push({
            title: 'Phase 2: Parse JSON Documents',
            detail: 'Read JSON metadata from each vector result to find graph IDs',
            status: 'active',
        });
        state.jsonHighlighted.add('v1');
        state.jsonHighlighted.add('v2');
        state.jsonHighlighted.add('v3');
        state.jsonHighlighted.add('v4');
        state.operations += 4; // one parse per result
        state.systemBoundaries += 1; // cross from vector to JSON
        updatePhasesUI();
    });
    // Phase 3: Switch to graph DB, find entry nodes
    steps.push(() => {
        state.phases[1].status = 'done';
        state.phases.push({
            title: 'Phase 3: Switch to Graph Database',
            detail: 'Use graphId from JSON to locate nodes in RDF store',
            status: 'active',
        });
        state.activeGraphNodes.add('g-john1');
        state.activeGraphNodes.add('g-jean1');
        state.activeGraphNodes.add('g-johan1');
        state.activeGraphNodes.add('g-juan1');
        state.operations += 4;
        state.systemBoundaries += 1;
        updatePhasesUI();
    });
    // Phase 4: First hop - father
    steps.push(() => {
        state.phases[2].status = 'done';
        state.phases.push({
            title: 'Phase 4: Graph Hop 1 — :hasFather',
            detail: 'Traverse :hasFather from each person to their father',
            status: 'active',
        });
        for (const id of ['g-john1', 'g-jean1', 'g-johan1', 'g-juan1']) {
            const p = graphPeople.get(id);
            if (p.fatherId) {
                state.traversedGraphEdges.add(`${id}->${p.fatherId}`);
                state.activeGraphNodes.add(p.fatherId);
            }
        }
        state.operations += 4;
        updatePhasesUI();
    });
    // Phase 5: Second hop - grandfather
    steps.push(() => {
        state.phases[3].status = 'done';
        state.phases.push({
            title: 'Phase 5: Graph Hop 2 — :hasFather',
            detail: 'Traverse :hasFather from each father to grandfather',
            status: 'active',
        });
        for (const id of ['g-robert1', 'g-pierre1', 'g-erik1', 'g-carlos1']) {
            const p = graphPeople.get(id);
            if (p.fatherId) {
                state.traversedGraphEdges.add(`${id}->${p.fatherId}`);
                state.activeGraphNodes.add(p.fatherId);
            }
        }
        state.operations += 4;
        updatePhasesUI();
    });
    // Phase 6: Third hop - great-grandfather
    steps.push(() => {
        state.phases[4].status = 'done';
        state.phases.push({
            title: 'Phase 6: Graph Hop 3 — :hasFather',
            detail: 'Traverse :hasFather from each grandfather to great-grandfather',
            status: 'active',
        });
        for (const id of ['g-william1', 'g-louis1', 'g-olaf1', 'g-miguel1']) {
            const p = graphPeople.get(id);
            if (p.fatherId) {
                state.traversedGraphEdges.add(`${id}->${p.fatherId}`);
                state.activeGraphNodes.add(p.fatherId);
            }
        }
        state.operations += 4;
        state.results = [
            { person: 'John Smith', greatgrandfather: 'Henry Smith', chain: 'John → Robert → William → Henry' },
            { person: 'Jean Dupont', greatgrandfather: 'Francois Dupont', chain: 'Jean → Pierre → Louis → Francois' },
            { person: 'Johan Berg', greatgrandfather: 'Sven Berg', chain: 'Johan → Erik → Olaf → Sven' },
            { person: 'Juan Garcia', greatgrandfather: 'Antonio Garcia', chain: 'Juan → Carlos → Miguel → Antonio' },
        ];
        state.phase = 'done';
        updatePhasesUI();
        updateCostUI();
        updateResultsUI();
    });
    return steps;
}
function updatePhasesUI() {
    const container = document.getElementById('phases-container');
    container.innerHTML = '';
    for (const phase of state.phases) {
        const card = document.createElement('div');
        card.className = `phase-card ${phase.status}`;
        const colorMap = { active: '#7c8cf8', done: '#34d399', error: '#f87c7c', pending: '#666880' };
        card.innerHTML = `
      <div class="phase-title" style="color: ${colorMap[phase.status]}">${phase.title}</div>
      <div class="phase-detail">${phase.detail}</div>
    `;
        container.appendChild(card);
    }
    updateCostUI();
}
function updateCostUI() {
    const container = document.getElementById('cost-container');
    container.innerHTML = `
    <div class="cost-bar">
      <span style="color:#9898ac;font-size:0.78rem;min-width:120px;">Operations: <strong>${state.operations}</strong></span>
      <div class="bar-track"><div class="bar-fill" style="width:${Math.min(100, state.operations * 4)}%;background:#facc15;"></div></div>
    </div>
    <div class="cost-bar">
      <span style="color:#9898ac;font-size:0.78rem;min-width:120px;">Boundaries: <strong>${state.systemBoundaries}</strong></span>
      <div class="bar-track"><div class="bar-fill" style="width:${state.systemBoundaries * 25}%;background:#f87c7c;"></div></div>
    </div>
  `;
}
function updateResultsUI() {
    // Append results after cost
    const container = document.getElementById('phases-container');
    if (state.results.length === 0)
        return;
    const div = document.createElement('div');
    div.style.marginTop = '12px';
    const label = document.createElement('div');
    label.style.cssText = 'font-size:0.6rem;color:#666880;text-transform:uppercase;letter-spacing:0.1em;margin-bottom:6px;margin-top:4px;';
    label.textContent = 'Final Results';
    div.appendChild(label);
    for (const r of state.results) {
        const card = document.createElement('div');
        card.className = 'phase-card done';
        card.innerHTML = `
      <div class="phase-title" style="color:#34d399">${r.person} → <strong>${r.greatgrandfather}</strong></div>
      <div class="phase-detail">${r.chain}</div>
    `;
        div.appendChild(card);
    }
    container.appendChild(div);
}
function resetState() {
    state = createInitialState();
    document.getElementById('phases-container').innerHTML = '';
    updateCostUI();
    draw();
}
function runHybrid() {
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
        setTimeout(playNext, 900);
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
    document.getElementById('btn-run').addEventListener('click', runHybrid);
    document.getElementById('btn-reset').addEventListener('click', resetState);
    document.getElementById('btn-step').addEventListener('click', stepOnce);
    window.addEventListener('resize', resize);
    resize();
    updateCostUI();
});
export {};
