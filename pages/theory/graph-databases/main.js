const COLORS = {
    bg: '#0a0a0f',
    edge: '#1e1e2a',
    edgeLabel: '#666880',
    node: '#7c8cf8',
    nodeText: '#d0d0dc',
    matched: '#34d399',
    active: '#facc15',
    fatherEdge: '#f87c7c',
    nameEdge: '#34d399',
    traversed: '#7c8cf8',
    dimNode: '#2a2a3a',
};
// --- Data ---
const graphNodes = new Map();
const triples = [];
function addPerson(id, label, x, y) {
    graphNodes.set(id, { id, label, pos: { x, y }, type: 'person' });
}
function addTriple(s, p, o) {
    triples.push({ subject: s, predicate: p, object: o });
}
// Family tree: Two "John" lineages
addPerson('john1', 'John Smith', 0.15, 0.12);
addPerson('robert1', 'Robert Smith', 0.15, 0.32);
addPerson('william1', 'William Smith', 0.15, 0.52);
addPerson('henry1', 'Henry Smith', 0.15, 0.72);
addPerson('john2', 'John Lee', 0.50, 0.12);
addPerson('david1', 'David Lee', 0.50, 0.32);
addPerson('james1', 'James Lee', 0.50, 0.52);
addPerson('thomas1', 'Thomas Lee', 0.50, 0.72);
// Non-John person for contrast
addPerson('jean1', 'Jean Dupont', 0.85, 0.12);
addPerson('pierre1', 'Pierre Dupont', 0.85, 0.32);
addPerson('louis1', 'Louis Dupont', 0.85, 0.52);
addPerson('francois1', 'Francois Dupont', 0.85, 0.72);
// :hasName triples
addTriple('john1', ':hasName', '"John"');
addTriple('john2', ':hasName', '"John"');
addTriple('jean1', ':hasName', '"Jean"');
addTriple('robert1', ':hasName', '"Robert"');
addTriple('william1', ':hasName', '"William"');
addTriple('henry1', ':hasName', '"Henry"');
addTriple('david1', ':hasName', '"David"');
addTriple('james1', ':hasName', '"James"');
addTriple('thomas1', ':hasName', '"Thomas"');
addTriple('pierre1', ':hasName', '"Pierre"');
addTriple('louis1', ':hasName', '"Louis"');
addTriple('francois1', ':hasName', '"Francois"');
// :hasFather triples
addTriple('john1', ':hasFather', 'robert1');
addTriple('robert1', ':hasFather', 'william1');
addTriple('william1', ':hasFather', 'henry1');
addTriple('john2', ':hasFather', 'david1');
addTriple('david1', ':hasFather', 'james1');
addTriple('james1', ':hasFather', 'thomas1');
addTriple('jean1', ':hasFather', 'pierre1');
addTriple('pierre1', ':hasFather', 'louis1');
addTriple('louis1', ':hasFather', 'francois1');
let state = {
    phase: 'idle',
    running: false,
    matchedTriples: new Set(),
    activeNodes: new Set(),
    traversedEdges: new Set(),
    bindings: [],
    currentBinding: -1,
    steps: [],
    stepQueue: [],
    stepIndex: 0,
};
let canvas;
let ctx;
let W = 520;
let H = 480;
function toScreen(nx, ny) {
    const mx = 60, my = 50;
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
function drawArrow(from, to, color, lineWidth, label) {
    const dx = to.x - from.x;
    const dy = to.y - from.y;
    const len = Math.sqrt(dx * dx + dy * dy);
    if (len < 1)
        return;
    const nx = dx / len;
    const ny = dy / len;
    const startR = 14;
    const endR = 18;
    const sx = from.x + nx * startR;
    const sy = from.y + ny * startR;
    const ex = to.x - nx * endR;
    const ey = to.y - ny * endR;
    ctx.strokeStyle = color;
    ctx.lineWidth = lineWidth;
    ctx.beginPath();
    ctx.moveTo(sx, sy);
    ctx.lineTo(ex, ey);
    ctx.stroke();
    // Arrowhead
    const headLen = 8;
    const angle = Math.atan2(ey - sy, ex - sx);
    ctx.fillStyle = color;
    ctx.beginPath();
    ctx.moveTo(ex, ey);
    ctx.lineTo(ex - headLen * Math.cos(angle - 0.4), ey - headLen * Math.sin(angle - 0.4));
    ctx.lineTo(ex - headLen * Math.cos(angle + 0.4), ey - headLen * Math.sin(angle + 0.4));
    ctx.closePath();
    ctx.fill();
    // Label
    if (label) {
        const mx = (sx + ex) / 2;
        const my = (sy + ey) / 2;
        ctx.fillStyle = color;
        ctx.font = '9px "Cascadia Code", "Fira Code", monospace';
        ctx.textAlign = 'center';
        ctx.textBaseline = 'bottom';
        // Offset perpendicular to edge
        const px = -ny * 10;
        const py = nx * 10;
        ctx.fillText(label, mx + px, my + py - 2);
    }
}
function draw() {
    ctx.fillStyle = COLORS.bg;
    ctx.fillRect(0, 0, W, H);
    // Draw :hasFather edges
    for (const t of triples) {
        if (t.predicate !== ':hasFather')
            continue;
        const sNode = graphNodes.get(t.subject);
        const oNode = graphNodes.get(t.object);
        if (!sNode || !oNode)
            continue;
        const from = toScreen(sNode.pos.x, sNode.pos.y);
        const to = toScreen(oNode.pos.x, oNode.pos.y);
        const edgeKey = `${t.subject}->${t.object}`;
        const isTraversed = state.traversedEdges.has(edgeKey);
        drawArrow(from, to, isTraversed ? COLORS.fatherEdge : COLORS.edge, isTraversed ? 2.5 : 1, ':hasFather');
    }
    // Draw nodes
    for (const node of graphNodes.values()) {
        const p = toScreen(node.pos.x, node.pos.y);
        const isActive = state.activeNodes.has(node.id);
        let color = COLORS.node;
        let radius = 12;
        if (isActive) {
            color = COLORS.matched;
            radius = 14;
            // Glow
            ctx.beginPath();
            ctx.arc(p.x, p.y, radius + 6, 0, Math.PI * 2);
            ctx.fillStyle = color + '33';
            ctx.fill();
        }
        else if (state.phase !== 'idle' && !state.activeNodes.has(node.id) && state.activeNodes.size > 0) {
            color = COLORS.dimNode;
        }
        ctx.beginPath();
        ctx.arc(p.x, p.y, radius, 0, Math.PI * 2);
        ctx.fillStyle = color;
        ctx.fill();
        // Label
        ctx.fillStyle = isActive ? '#ffffff' : COLORS.nodeText;
        ctx.font = `${isActive ? 'bold ' : ''}10px "Segoe UI", system-ui, sans-serif`;
        ctx.textAlign = 'center';
        ctx.textBaseline = 'bottom';
        ctx.fillText(node.label, p.x, p.y - radius - 3);
        // Name tag
        const nameTriple = triples.find(t => t.subject === node.id && t.predicate === ':hasName');
        if (nameTriple) {
            ctx.fillStyle = isActive ? COLORS.nameEdge : COLORS.edgeLabel;
            ctx.font = '9px "Cascadia Code", monospace';
            ctx.textBaseline = 'top';
            ctx.fillText(nameTriple.object, p.x, p.y + radius + 3);
        }
    }
}
function buildQuerySteps() {
    const steps = [];
    // Step 1: Find all ?person with :hasName "John"
    steps.push(() => {
        state.activeNodes.add('john1');
        state.activeNodes.add('john2');
        state.steps.push({
            description: 'Pattern: ?person :hasName "John"',
            detail: 'Found 2 matches: John Smith, John Lee',
            active: true,
            done: false,
        });
        updateStepsUI();
    });
    // Step 2: john1 :hasFather -> robert1
    steps.push(() => {
        state.steps[0].active = false;
        state.steps[0].done = true;
        state.activeNodes.add('robert1');
        state.traversedEdges.add('john1->robert1');
        state.activeNodes.add('david1');
        state.traversedEdges.add('john2->david1');
        state.steps.push({
            description: 'Pattern: ?person :hasFather ?father',
            detail: 'John Smith → Robert Smith, John Lee → David Lee',
            active: true,
            done: false,
        });
        updateStepsUI();
    });
    // Step 3: father :hasFather -> grandfather
    steps.push(() => {
        state.steps[1].active = false;
        state.steps[1].done = true;
        state.activeNodes.add('william1');
        state.traversedEdges.add('robert1->william1');
        state.activeNodes.add('james1');
        state.traversedEdges.add('david1->james1');
        state.steps.push({
            description: 'Pattern: ?father :hasFather ?grandfather',
            detail: 'Robert Smith → William Smith, David Lee → James Lee',
            active: true,
            done: false,
        });
        updateStepsUI();
    });
    // Step 4: grandfather :hasFather -> greatgrandfather
    steps.push(() => {
        state.steps[2].active = false;
        state.steps[2].done = true;
        state.activeNodes.add('henry1');
        state.traversedEdges.add('william1->henry1');
        state.activeNodes.add('thomas1');
        state.traversedEdges.add('james1->thomas1');
        state.steps.push({
            description: 'Pattern: ?grandfather :hasFather ?greatgrandfather',
            detail: 'William Smith → Henry Smith, James Lee → Thomas Lee',
            active: true,
            done: false,
        });
        state.bindings = [
            { person: 'John Smith', father: 'Robert Smith', grandfather: 'William Smith', greatgrandfather: 'Henry Smith' },
            { person: 'John Lee', father: 'David Lee', grandfather: 'James Lee', greatgrandfather: 'Thomas Lee' },
        ];
        state.phase = 'done';
        updateStepsUI();
        updateResultsUI();
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
function updateResultsUI() {
    const container = document.getElementById('results-container');
    container.innerHTML = '';
    if (state.bindings.length === 0) {
        container.innerHTML = '<div class="explanation">Click "Run Query" to execute the SPARQL query</div>';
        return;
    }
    for (const b of state.bindings) {
        const card = document.createElement('div');
        card.className = 'result-card';
        card.innerHTML = `
      <div style="font-weight:600;color:#e8e8f0;">${b.person}'s great-grandfather: <span class="highlight-green">${b.greatgrandfather}</span></div>
      <div class="chain">${b.person} → ${b.father} → ${b.grandfather} → ${b.greatgrandfather}</div>
    `;
        container.appendChild(card);
    }
}
function resetState() {
    state = {
        phase: 'idle',
        running: false,
        matchedTriples: new Set(),
        activeNodes: new Set(),
        traversedEdges: new Set(),
        bindings: [],
        currentBinding: -1,
        steps: [],
        stepQueue: [],
        stepIndex: 0,
    };
    document.getElementById('steps-container').innerHTML = '';
    updateResultsUI();
    draw();
}
function runQuery() {
    resetState();
    state.phase = 'running';
    state.stepQueue = buildQuerySteps();
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
        setTimeout(playNext, 1000);
    }
}
function stepOnce() {
    if (state.phase === 'idle') {
        state.phase = 'running';
        state.stepQueue = buildQuerySteps();
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
    document.getElementById('btn-query').addEventListener('click', runQuery);
    document.getElementById('btn-reset').addEventListener('click', resetState);
    document.getElementById('btn-step').addEventListener('click', stepOnce);
    window.addEventListener('resize', resize);
    resize();
});
export {};
