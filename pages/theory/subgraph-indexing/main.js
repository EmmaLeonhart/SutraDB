const COLORS = {
    bg: '#0a0a0f',
    edge: '#1e1e2a',
    edgeLabel: '#666880',
    node: '#7c8cf8',
    nodeText: '#d0d0dc',
    matched: '#34d399',
    active: '#facc15',
    predEdge: '#f87c7c',
    dimNode: '#2a2a3a',
    column: '#7c8cf8',
    scanHighlight: '#34d399',
    simdBlock: '#facc15',
};
const graphNodes = new Map();
const triples = [];
function addNode(id, label, x, y, type) {
    graphNodes.set(id, { id, label, pos: { x, y }, type });
}
function addTriple(s, p, o) {
    triples.push({ subject: s, predicate: p, object: o });
}
// 4 countries with identical subgraph shape: country → capital → mayor → birthDate
addNode('jp', 'Japan', 0.10, 0.15, 'country');
addNode('tokyo', 'Tokyo', 0.30, 0.15, 'city');
addNode('koike', 'Koike Yuriko', 0.50, 0.15, 'person');
addNode('koike_bd', '1952-07-15', 0.70, 0.15, 'date');
addNode('fr', 'France', 0.10, 0.40, 'country');
addNode('paris', 'Paris', 0.30, 0.40, 'city');
addNode('hidalgo', 'Anne Hidalgo', 0.50, 0.40, 'person');
addNode('hidalgo_bd', '1959-06-19', 0.70, 0.40, 'date');
addNode('de', 'Germany', 0.10, 0.65, 'country');
addNode('berlin', 'Berlin', 0.30, 0.65, 'city');
addNode('wegner', 'Kai Wegner', 0.50, 0.65, 'person');
addNode('wegner_bd', '1972-10-17', 0.70, 0.65, 'date');
addNode('uk', 'UK', 0.10, 0.90, 'country');
addNode('london', 'London', 0.30, 0.90, 'city');
addNode('khan', 'Sadiq Khan', 0.50, 0.90, 'person');
addNode('khan_bd', '1970-10-08', 0.70, 0.90, 'date');
// Triples for each subgraph
for (const [country, capital, mayor, bd] of [
    ['jp', 'tokyo', 'koike', 'koike_bd'],
    ['fr', 'paris', 'hidalgo', 'hidalgo_bd'],
    ['de', 'berlin', 'wegner', 'wegner_bd'],
    ['uk', 'london', 'khan', 'khan_bd'],
]) {
    addTriple(country, ':hasCapital', capital);
    addTriple(capital, ':hasMayor', mayor);
    addTriple(mayor, ':birthDate', bd);
}
const pseudoTable = [
    { country: 'Japan', capital: 'Tokyo', mayor: 'Koike Yuriko', birthDate: '1952-07-15' },
    { country: 'France', capital: 'Paris', mayor: 'Anne Hidalgo', birthDate: '1959-06-19' },
    { country: 'Germany', capital: 'Berlin', mayor: 'Kai Wegner', birthDate: '1972-10-17' },
    { country: 'UK', capital: 'London', mayor: 'Sadiq Khan', birthDate: '1970-10-08' },
];
let state = createInitialState();
function createInitialState() {
    return {
        phase: 'idle',
        running: false,
        activeEdges: new Set(),
        activeNodes: new Set(),
        showTable: false,
        scanColumn: -1,
        scanRow: -1,
        matchedRows: new Set(),
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
    const mx = 50, my = 40;
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
    const startR = 12;
    const endR = 16;
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
    const headLen = 7;
    const angle = Math.atan2(ey - sy, ex - sx);
    ctx.fillStyle = color;
    ctx.beginPath();
    ctx.moveTo(ex, ey);
    ctx.lineTo(ex - headLen * Math.cos(angle - 0.4), ey - headLen * Math.sin(angle - 0.4));
    ctx.lineTo(ex - headLen * Math.cos(angle + 0.4), ey - headLen * Math.sin(angle + 0.4));
    ctx.closePath();
    ctx.fill();
    if (label) {
        const mx2 = (sx + ex) / 2;
        const my2 = (sy + ey) / 2;
        ctx.fillStyle = color;
        ctx.font = '8px "Cascadia Code", "Fira Code", monospace';
        ctx.textAlign = 'center';
        ctx.textBaseline = 'bottom';
        ctx.fillText(label, mx2, my2 - 4);
    }
}
function drawGraph() {
    ctx.fillStyle = COLORS.bg;
    ctx.fillRect(0, 0, W, H);
    // Draw edges
    for (const t of triples) {
        const sNode = graphNodes.get(t.subject);
        const oNode = graphNodes.get(t.object);
        if (!sNode || !oNode)
            continue;
        const from = toScreen(sNode.pos.x, sNode.pos.y);
        const to = toScreen(oNode.pos.x, oNode.pos.y);
        const edgeKey = `${t.subject}->${t.object}`;
        const isActive = state.activeEdges.has(edgeKey);
        drawArrow(from, to, isActive ? COLORS.predEdge : COLORS.edge, isActive ? 2 : 1, t.predicate);
    }
    // Draw nodes
    const typeColors = {
        country: '#f87c7c',
        city: '#facc15',
        person: '#7c8cf8',
        date: '#34d399',
    };
    for (const node of graphNodes.values()) {
        const p = toScreen(node.pos.x, node.pos.y);
        const isActive = state.activeNodes.has(node.id);
        const color = isActive ? COLORS.matched : (typeColors[node.type] || COLORS.node);
        const radius = isActive ? 11 : 9;
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
        ctx.fillStyle = isActive ? '#ffffff' : COLORS.nodeText;
        ctx.font = `${isActive ? 'bold ' : ''}9px "Segoe UI", system-ui, sans-serif`;
        ctx.textAlign = 'center';
        ctx.textBaseline = 'bottom';
        ctx.fillText(node.label, p.x, p.y - radius - 2);
    }
    // Type legend
    ctx.font = '9px "Segoe UI", system-ui, sans-serif';
    let ly = 14;
    for (const [type, col] of Object.entries(typeColors)) {
        ctx.fillStyle = col;
        ctx.beginPath();
        ctx.arc(12, ly, 4, 0, Math.PI * 2);
        ctx.fill();
        ctx.fillStyle = COLORS.nodeText;
        ctx.textAlign = 'left';
        ctx.textBaseline = 'middle';
        ctx.fillText(type, 20, ly);
        ly += 16;
    }
}
function drawTable() {
    ctx.fillStyle = COLORS.bg;
    ctx.fillRect(0, 0, W, H);
    const headers = ['country', 'capital', 'mayor', 'birthDate'];
    const colW = (W - 40) / 4;
    const rowH = 42;
    const startX = 20;
    const startY = 50;
    // Title
    ctx.fillStyle = COLORS.matched;
    ctx.font = 'bold 12px "Segoe UI", system-ui, sans-serif';
    ctx.textAlign = 'left';
    ctx.textBaseline = 'top';
    ctx.fillText('Pseudo-Table: country \u2192 capital \u2192 mayor \u2192 birthDate', startX, 14);
    ctx.fillStyle = '#666880';
    ctx.font = '9px "Segoe UI", system-ui, sans-serif';
    ctx.fillText('Contiguous columnar memory \u2014 SIMD scans 4 values per cycle', startX, 32);
    // Draw column headers
    for (let c = 0; c < headers.length; c++) {
        const x = startX + c * colW;
        const isScanning = state.scanColumn === c;
        // Column background
        if (isScanning) {
            ctx.fillStyle = COLORS.column + '15';
            ctx.fillRect(x, startY - 2, colW - 4, rowH * (pseudoTable.length + 1) + 4);
        }
        ctx.fillStyle = isScanning ? COLORS.column : '#666880';
        ctx.font = `${isScanning ? 'bold ' : ''}10px "Cascadia Code", monospace`;
        ctx.textAlign = 'left';
        ctx.textBaseline = 'middle';
        ctx.fillText(headers[c], x + 4, startY + rowH / 2);
    }
    // Draw rows
    for (let r = 0; r < pseudoTable.length; r++) {
        const row = pseudoTable[r];
        const y = startY + (r + 1) * rowH;
        const values = [row.country, row.capital, row.mayor, row.birthDate];
        const isMatched = state.matchedRows.has(r);
        const isSIMDRow = state.scanRow === r;
        // Row highlight
        if (isMatched) {
            ctx.fillStyle = COLORS.scanHighlight + '22';
            ctx.fillRect(startX, y - 2, W - 40, rowH);
        }
        // SIMD block highlight
        if (isSIMDRow && state.scanColumn >= 0) {
            const sx = startX + state.scanColumn * colW;
            ctx.fillStyle = COLORS.simdBlock + '33';
            ctx.fillRect(sx, y - 2, colW - 4, rowH);
            ctx.strokeStyle = COLORS.simdBlock;
            ctx.lineWidth = 1.5;
            ctx.strokeRect(sx, y - 2, colW - 4, rowH);
        }
        for (let c = 0; c < values.length; c++) {
            const x = startX + c * colW;
            ctx.fillStyle = isMatched ? COLORS.scanHighlight : COLORS.nodeText;
            ctx.font = '9px "Cascadia Code", monospace';
            ctx.textAlign = 'left';
            ctx.textBaseline = 'middle';
            // Truncate long values
            let val = values[c];
            if (val.length > 12)
                val = val.substring(0, 11) + '\u2026';
            ctx.fillText(val, x + 4, y + rowH / 2);
        }
    }
    // SIMD annotation
    if (state.phase === 'simd-scan') {
        const annotY = startY + (pseudoTable.length + 1) * rowH + 20;
        ctx.fillStyle = COLORS.simdBlock;
        ctx.font = 'bold 10px "Cascadia Code", monospace';
        ctx.textAlign = 'left';
        ctx.fillText('AVX2: comparing 4 u64 TermIDs per cycle', startX, annotY);
        ctx.fillStyle = '#666880';
        ctx.font = '9px "Segoe UI", system-ui, sans-serif';
        ctx.fillText('No pointer-chasing \u2014 data is contiguous in memory', startX, annotY + 16);
    }
    // Depth threshold annotation
    if (state.phase === 'done') {
        const annotY = startY + (pseudoTable.length + 1) * rowH + 20;
        ctx.fillStyle = COLORS.matched;
        ctx.font = 'bold 10px "Segoe UI", system-ui, sans-serif';
        ctx.textAlign = 'left';
        ctx.fillText('Depth 3 subgraph \u2192 threshold: base \u00d7 9 (geometric)', startX, annotY);
        ctx.fillStyle = '#666880';
        ctx.font = '9px "Segoe UI", system-ui, sans-serif';
        ctx.fillText('Tree-like pattern (fan-in \u2248 1) \u2014 ideal for materialization', startX, annotY + 16);
        ctx.fillText('3 joins eliminated per query \u2192 single columnar scan', startX, annotY + 30);
    }
}
function draw() {
    if (state.showTable) {
        drawTable();
    }
    else {
        drawGraph();
    }
}
function buildSteps() {
    const steps = [];
    // Step 1: Highlight the repeating subgraph pattern
    steps.push(() => {
        state.phase = 'graph-scan';
        // Highlight first subgraph
        state.activeNodes.add('jp');
        state.activeNodes.add('tokyo');
        state.activeNodes.add('koike');
        state.activeNodes.add('koike_bd');
        state.activeEdges.add('jp->tokyo');
        state.activeEdges.add('tokyo->koike');
        state.activeEdges.add('koike->koike_bd');
        state.steps.push({
            description: 'Pattern detected: country \u2192 capital \u2192 mayor \u2192 birthDate',
            detail: 'Japan \u2192 Tokyo \u2192 Koike Yuriko \u2192 1952-07-15. Scanning for more instances...',
            active: true, done: false,
        });
        updateStepsUI();
    });
    // Step 2: Find all 4 instances
    steps.push(() => {
        state.steps[0].active = false;
        state.steps[0].done = true;
        // Highlight all subgraphs
        for (const id of graphNodes.keys()) {
            state.activeNodes.add(id);
        }
        for (const t of triples) {
            state.activeEdges.add(`${t.subject}->${t.object}`);
        }
        state.phase = 'discovery';
        state.steps.push({
            description: 'Found 4 instances of identical subgraph shape',
            detail: 'All 4 share the same 3-hop pattern. Depth 3 \u2192 geometric threshold = base \u00d7 9. Pattern qualifies!',
            active: true, done: false,
        });
        updateStepsUI();
    });
    // Step 3: Materialize as columnar pseudo-table
    steps.push(() => {
        state.steps[1].active = false;
        state.steps[1].done = true;
        state.showTable = true;
        state.phase = 'columnar';
        state.steps.push({
            description: 'Materialized as columnar pseudo-table',
            detail: '4 rows \u00d7 4 columns. Each column is contiguous u64 TermIDs in memory. 3 joins eliminated.',
            active: true, done: false,
        });
        updateStepsUI();
    });
    // Step 4: SIMD scan on birthDate column
    steps.push(() => {
        state.steps[2].active = false;
        state.steps[2].done = true;
        state.phase = 'simd-scan';
        state.scanColumn = 3; // birthDate
        state.scanRow = 0;
        state.steps.push({
            description: 'SIMD scan: FILTER(?birthDate < "1960-01-01")',
            detail: 'AVX2 compares 4 TermIDs per cycle. Scanning birthDate column...',
            active: true, done: false,
        });
        updateStepsUI();
    });
    // Step 5: SIMD scan completes — matched rows
    steps.push(() => {
        state.steps[3].active = false;
        state.steps[3].done = true;
        state.scanRow = -1;
        state.matchedRows.add(0); // Japan - 1952
        state.matchedRows.add(1); // France - 1959
        state.phase = 'done';
        state.steps.push({
            description: 'Scan complete: 2 matches (born before 1960)',
            detail: 'Japan/Koike (1952) and France/Hidalgo (1959). Zero pointer-chasing \u2014 pure columnar scan.',
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
    state.phase = 'graph-scan';
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
        state.phase = 'graph-scan';
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
    document.getElementById('btn-run').addEventListener('click', runDemo);
    document.getElementById('btn-reset').addEventListener('click', resetState);
    document.getElementById('btn-step').addEventListener('click', stepOnce);
    window.addEventListener('resize', resize);
    resize();
});
export {};
