const COLORS = {
    bg: '#0a0a0f',
    edge: '#1e1e2a',
    node: '#7c8cf8',
    nodeText: '#d0d0dc',
    hotNode: '#f87c7c',
    warmNode: '#facc15',
    coldNode: '#2a2a3a',
    entryNode: '#34d399',
    traversalPath: '#7c8cf8',
    dimNode: '#2a2a3a',
};
const nodes = new Map();
const edges = [];
function addNode(id, label, x, y, pr, tc) {
    nodes.set(id, { id, label, pos: { x, y }, pageRank: pr, traversalCount: tc, isEntry: false });
}
function addEdge(from, to, pred) {
    edges.push({ from, to, predicate: pred });
}
// Hub-and-spoke graph: Tokyo is a hub (high PageRank)
addNode('tokyo', 'Tokyo', 0.50, 0.45, 0.92, 847);
addNode('shibuya', 'Shibuya', 0.30, 0.25, 0.35, 312);
addNode('shinjuku', 'Shinjuku', 0.65, 0.22, 0.38, 289);
addNode('ginza', 'Ginza', 0.75, 0.45, 0.25, 198);
addNode('akiba', 'Akihabara', 0.35, 0.60, 0.22, 167);
addNode('japan', 'Japan', 0.50, 0.12, 0.85, 723);
addNode('kanto', 'Kanto', 0.20, 0.42, 0.45, 401);
addNode('osaka', 'Osaka', 0.85, 0.70, 0.65, 534);
addNode('kyoto', 'Kyoto', 0.65, 0.80, 0.58, 489);
addNode('namba', 'Namba', 0.90, 0.88, 0.18, 123);
addNode('fushimi', 'Fushimi Inari', 0.50, 0.90, 0.20, 156);
addNode('nara', 'Nara', 0.78, 0.92, 0.30, 201);
// Cold nodes — rarely traversed
addNode('tottori', 'Tottori', 0.10, 0.80, 0.05, 12);
addNode('shimane', 'Shimane', 0.10, 0.92, 0.03, 8);
// Edges — hub-and-spoke around Tokyo and Osaka
addEdge('japan', 'tokyo', ':hasCapital');
addEdge('japan', 'kanto', ':hasRegion');
addEdge('kanto', 'tokyo', ':contains');
addEdge('tokyo', 'shibuya', ':hasDistrict');
addEdge('tokyo', 'shinjuku', ':hasDistrict');
addEdge('tokyo', 'ginza', ':hasDistrict');
addEdge('tokyo', 'akiba', ':hasDistrict');
addEdge('japan', 'osaka', ':hasPrefecture');
addEdge('japan', 'kyoto', ':hasPrefecture');
addEdge('osaka', 'namba', ':hasDistrict');
addEdge('kyoto', 'fushimi', ':hasSite');
addEdge('kyoto', 'nara', ':nearCity');
addEdge('osaka', 'nara', ':nearCity');
// Cold edges
addEdge('japan', 'tottori', ':hasPrefecture');
addEdge('tottori', 'shimane', ':neighbor');
let state = createInitialState();
function createInitialState() {
    return {
        phase: 'idle',
        running: false,
        showPageRank: false,
        showCounters: false,
        selectedEntries: new Set(),
        traversalPath: [],
        activeEdges: new Set(),
        hotZone: new Set(),
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
    const mx = 45, my = 35;
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
    // Draw hot zone glow
    if (state.hotZone.size > 0) {
        for (const id of state.hotZone) {
            const node = nodes.get(id);
            const p = toScreen(node.pos.x, node.pos.y);
            const radius = 30 + node.traversalCount / 20;
            const grad = ctx.createRadialGradient(p.x, p.y, 0, p.x, p.y, radius);
            grad.addColorStop(0, COLORS.hotNode + '18');
            grad.addColorStop(1, COLORS.hotNode + '00');
            ctx.fillStyle = grad;
            ctx.beginPath();
            ctx.arc(p.x, p.y, radius, 0, Math.PI * 2);
            ctx.fill();
        }
    }
    // Draw edges
    for (const edge of edges) {
        const sNode = nodes.get(edge.from);
        const oNode = nodes.get(edge.to);
        const from = toScreen(sNode.pos.x, sNode.pos.y);
        const to = toScreen(oNode.pos.x, oNode.pos.y);
        const edgeKey = `${edge.from}->${edge.to}`;
        const isActive = state.activeEdges.has(edgeKey);
        ctx.strokeStyle = isActive ? COLORS.traversalPath : COLORS.edge;
        ctx.lineWidth = isActive ? 2.5 : 0.8;
        ctx.beginPath();
        ctx.moveTo(from.x, from.y);
        ctx.lineTo(to.x, to.y);
        ctx.stroke();
        // Arrow
        if (isActive) {
            const dx = to.x - from.x;
            const dy = to.y - from.y;
            const len = Math.sqrt(dx * dx + dy * dy);
            const nx = dx / len;
            const ny = dy / len;
            const ex = to.x - nx * 12;
            const ey = to.y - ny * 12;
            ctx.fillStyle = COLORS.traversalPath;
            ctx.beginPath();
            ctx.moveTo(ex + ny * 4, ey - nx * 4);
            ctx.lineTo(to.x - nx * 8, to.y - ny * 8);
            ctx.lineTo(ex - ny * 4, ey + nx * 4);
            ctx.closePath();
            ctx.fill();
        }
    }
    // Draw nodes
    for (const node of nodes.values()) {
        const p = toScreen(node.pos.x, node.pos.y);
        const isEntry = state.selectedEntries.has(node.id);
        const isHot = state.hotZone.has(node.id);
        const isOnPath = state.traversalPath.includes(node.id);
        // Size by PageRank when showing it
        let radius = 8;
        let color = COLORS.node;
        if (state.showPageRank) {
            radius = 5 + node.pageRank * 12;
            if (node.pageRank > 0.7)
                color = COLORS.hotNode;
            else if (node.pageRank > 0.3)
                color = COLORS.warmNode;
            else
                color = COLORS.coldNode;
        }
        if (state.showCounters) {
            if (node.traversalCount > 400)
                color = COLORS.hotNode;
            else if (node.traversalCount > 150)
                color = COLORS.warmNode;
            else
                color = COLORS.coldNode;
        }
        if (isEntry) {
            color = COLORS.entryNode;
            radius = Math.max(radius, 12);
        }
        if (isOnPath) {
            color = COLORS.traversalPath;
        }
        // Glow for entries
        if (isEntry || isHot) {
            ctx.beginPath();
            ctx.arc(p.x, p.y, radius + 5, 0, Math.PI * 2);
            ctx.fillStyle = color + '33';
            ctx.fill();
        }
        ctx.beginPath();
        ctx.arc(p.x, p.y, radius, 0, Math.PI * 2);
        ctx.fillStyle = color;
        ctx.fill();
        // Label
        ctx.fillStyle = (isEntry || isOnPath || node.pageRank > 0.3 || !state.showPageRank) ? COLORS.nodeText : '#555568';
        ctx.font = `${isEntry ? 'bold ' : ''}9px "Segoe UI", system-ui, sans-serif`;
        ctx.textAlign = 'center';
        ctx.textBaseline = 'bottom';
        ctx.fillText(node.label, p.x, p.y - radius - 2);
        // PageRank value
        if (state.showPageRank) {
            ctx.fillStyle = '#666880';
            ctx.font = '8px "Cascadia Code", monospace';
            ctx.textBaseline = 'top';
            ctx.fillText(`PR: ${node.pageRank.toFixed(2)}`, p.x, p.y + radius + 2);
        }
        // Traversal counter
        if (state.showCounters) {
            ctx.fillStyle = node.traversalCount > 400 ? COLORS.hotNode : '#666880';
            ctx.font = '8px "Cascadia Code", monospace';
            ctx.textBaseline = 'top';
            ctx.fillText(`${node.traversalCount}x`, p.x, p.y + radius + 2);
        }
    }
    // Legend
    if (state.showPageRank || state.showCounters) {
        ctx.font = '9px "Segoe UI", system-ui, sans-serif';
        ctx.textAlign = 'left';
        ctx.textBaseline = 'top';
        const items = state.showPageRank
            ? [['Hot (PR > 0.7)', COLORS.hotNode], ['Warm (> 0.3)', COLORS.warmNode], ['Cold', COLORS.coldNode]]
            : [['Hot (> 400x)', COLORS.hotNode], ['Warm (> 150x)', COLORS.warmNode], ['Cold', COLORS.coldNode]];
        let ly = 8;
        for (const [label, col] of items) {
            ctx.fillStyle = col;
            ctx.beginPath();
            ctx.arc(12, ly + 5, 4, 0, Math.PI * 2);
            ctx.fill();
            ctx.fillStyle = COLORS.nodeText;
            ctx.fillText(label, 20, ly);
            ly += 16;
        }
    }
}
function buildSteps() {
    const steps = [];
    // Step 1: Compute PageRank
    steps.push(() => {
        state.phase = 'pagerank';
        state.showPageRank = true;
        state.steps.push({
            description: 'PageRank computed across graph',
            detail: 'Tokyo (0.92) and Japan (0.85) are highest \u2014 most linked-to nodes. Tottori (0.05) and Shimane (0.03) are cold.',
            active: true, done: false,
        });
        updateStepsUI();
    });
    // Step 2: Select entry points
    steps.push(() => {
        state.steps[0].active = false;
        state.steps[0].done = true;
        state.phase = 'entry-select';
        state.selectedEntries.add('tokyo');
        state.selectedEntries.add('japan');
        state.selectedEntries.add('osaka');
        state.steps.push({
            description: 'Top-K PageRank nodes become traversal entry points',
            detail: 'Japan, Tokyo, Osaka selected. Traversals start here instead of random entry \u2014 fewer hops to reach any node.',
            active: true, done: false,
        });
        updateStepsUI();
    });
    // Step 3: Show traversal counters
    steps.push(() => {
        state.steps[1].active = false;
        state.steps[1].done = true;
        state.phase = 'counters';
        state.showPageRank = false;
        state.showCounters = true;
        state.hotZone.add('tokyo');
        state.hotZone.add('japan');
        state.hotZone.add('osaka');
        state.hotZone.add('kyoto');
        state.hotZone.add('kanto');
        state.steps.push({
            description: 'Traversal counters identify hot areas at runtime',
            detail: 'Tokyo (847x), Japan (723x), Osaka (534x) are busiest. These get materialized adjacency lists \u2014 all neighbors pre-loaded in contiguous memory.',
            active: true, done: false,
        });
        updateStepsUI();
    });
    // Step 4: Demo traversal from entry point
    steps.push(() => {
        state.steps[2].active = false;
        state.steps[2].done = true;
        state.phase = 'traversal';
        state.showCounters = false;
        state.hotZone.clear();
        // Traverse: Japan -> Tokyo -> Shibuya
        state.traversalPath = ['japan', 'tokyo', 'shibuya'];
        state.activeEdges.add('japan->tokyo');
        state.activeEdges.add('tokyo->shibuya');
        state.steps.push({
            description: 'Fast traversal: Japan \u2192 Tokyo \u2192 Shibuya',
            detail: 'Started at Japan (PageRank entry point). Tokyo\'s adjacency list is materialized \u2014 no B-tree lookup needed for its neighbors.',
            active: true, done: false,
        });
        updateStepsUI();
    });
    // Step 5: Summary
    steps.push(() => {
        state.steps[3].active = false;
        state.steps[3].done = true;
        state.phase = 'done';
        state.steps.push({
            description: 'Two complementary optimizations',
            detail: 'PageRank = where to START (structural importance). Traversal counters = what to CACHE (runtime hotness). Together they eliminate cold-start waste and index thrashing.',
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
