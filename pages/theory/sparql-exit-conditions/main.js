const COLORS = {
    bg: '#0a0a0f',
    edge: '#1e1e2a',
    node: '#7c8cf8',
    nodeText: '#d0d0dc',
    matched: '#34d399',
    active: '#facc15',
    stopped: '#f87c7c',
    traversed: '#7c8cf8',
    dimNode: '#2a2a3a',
    skipped: '#333340',
};
const nodes = new Map();
function addNode(id, label, x, y, year, diedInOffice) {
    nodes.set(id, { id, label, pos: { x, y }, diedInOffice, year });
}
// Chain of US presidents (simplified) — traverse successors until one died in office
addNode('wash', 'Washington', 0.08, 0.50, 1789, false);
addNode('adams', 'J. Adams', 0.18, 0.50, 1797, false);
addNode('jeff', 'Jefferson', 0.28, 0.50, 1801, false);
addNode('madi', 'Madison', 0.38, 0.50, 1809, false);
addNode('monr', 'Monroe', 0.48, 0.50, 1817, false);
addNode('jqa', 'J.Q. Adams', 0.58, 0.50, 1825, false);
addNode('jack', 'Jackson', 0.68, 0.50, 1829, false);
addNode('vbur', 'Van Buren', 0.78, 0.50, 1837, false);
addNode('wharr', 'W.H. Harrison', 0.88, 0.50, 1841, true); // died in office!
// Also show nodes that would be traversed without UNTIL
addNode('tyler', 'Tyler', 0.08, 0.80, 1841, false);
addNode('polk', 'Polk', 0.18, 0.80, 1845, false);
addNode('taylor', 'Taylor', 0.28, 0.80, 1849, true); // also died
addNode('fill', 'Fillmore', 0.38, 0.80, 1850, false);
addNode('pierce', 'Pierce', 0.48, 0.80, 1853, false);
addNode('buch', 'Buchanan', 0.58, 0.80, 1857, false);
addNode('linc', 'Lincoln', 0.68, 0.80, 1861, true); // died
// Successor chain
const chain = ['wash', 'adams', 'jeff', 'madi', 'monr', 'jqa', 'jack', 'vbur', 'wharr', 'tyler', 'polk', 'taylor', 'fill', 'pierce', 'buch', 'linc'];
let state = createInitialState();
function createInitialState() {
    return {
        phase: 'idle',
        running: false,
        mode: 'none',
        traversedNodes: new Set(),
        currentNode: '',
        exitNode: '',
        stoppedAt: '',
        skippedNodes: new Set(),
        traversedEdges: new Set(),
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
    const mx = 35, my = 40;
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
    // Draw successor edges
    for (let i = 0; i < chain.length - 1; i++) {
        const sNode = nodes.get(chain[i]);
        const oNode = nodes.get(chain[i + 1]);
        const from = toScreen(sNode.pos.x, sNode.pos.y);
        const to = toScreen(oNode.pos.x, oNode.pos.y);
        const edgeKey = `${chain[i]}->${chain[i + 1]}`;
        const isTraversed = state.traversedEdges.has(edgeKey);
        ctx.strokeStyle = isTraversed ? COLORS.traversed : COLORS.edge;
        ctx.lineWidth = isTraversed ? 2 : 0.8;
        ctx.beginPath();
        ctx.moveTo(from.x, from.y);
        ctx.lineTo(to.x, to.y);
        ctx.stroke();
        // Arrow
        const dx = to.x - from.x;
        const dy = to.y - from.y;
        const len = Math.sqrt(dx * dx + dy * dy);
        if (len > 1) {
            const nx = dx / len;
            const ny = dy / len;
            const ex = to.x - nx * 10;
            const ey = to.y - ny * 10;
            ctx.fillStyle = isTraversed ? COLORS.traversed : COLORS.edge;
            ctx.beginPath();
            ctx.moveTo(ex + ny * 3, ey - nx * 3);
            ctx.lineTo(to.x - nx * 6, to.y - ny * 6);
            ctx.lineTo(ex - ny * 3, ey + nx * 3);
            ctx.closePath();
            ctx.fill();
        }
    }
    // Draw nodes
    for (const node of nodes.values()) {
        const p = toScreen(node.pos.x, node.pos.y);
        const isTraversed = state.traversedNodes.has(node.id);
        const isCurrent = state.currentNode === node.id;
        const isExit = state.stoppedAt === node.id;
        const isSkipped = state.skippedNodes.has(node.id);
        let color = COLORS.node;
        let radius = 8;
        if (isExit) {
            color = COLORS.stopped;
            radius = 11;
        }
        else if (isCurrent) {
            color = COLORS.active;
            radius = 10;
        }
        else if (isTraversed) {
            color = COLORS.traversed;
            radius = 9;
        }
        else if (isSkipped) {
            color = COLORS.skipped;
            radius = 7;
        }
        else if (state.phase !== 'idle') {
            color = COLORS.dimNode;
            radius = 7;
        }
        // Died-in-office marker
        if (node.diedInOffice) {
            ctx.beginPath();
            ctx.arc(p.x, p.y, radius + 4, 0, Math.PI * 2);
            ctx.strokeStyle = COLORS.stopped + '66';
            ctx.lineWidth = 1.5;
            ctx.stroke();
        }
        // Glow
        if (isCurrent || isExit) {
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
        ctx.fillStyle = (isTraversed || isCurrent || isExit) ? '#ffffff' : (isSkipped ? '#555568' : COLORS.nodeText);
        ctx.font = `${(isCurrent || isExit) ? 'bold ' : ''}8px "Segoe UI", system-ui, sans-serif`;
        ctx.textAlign = 'center';
        ctx.textBaseline = 'bottom';
        ctx.fillText(node.label, p.x, p.y - radius - 2);
        // Year
        ctx.fillStyle = '#555568';
        ctx.font = '7px "Cascadia Code", monospace';
        ctx.textBaseline = 'top';
        ctx.fillText(String(node.year), p.x, p.y + radius + 2);
    }
    // Labels
    if (state.phase !== 'idle') {
        ctx.font = '9px "Segoe UI", system-ui, sans-serif';
        ctx.textAlign = 'left';
        ctx.textBaseline = 'top';
        ctx.fillStyle = COLORS.stopped;
        ctx.fillText('\u25CB = died in office', 8, 8);
        if (state.mode === 'with-until' && state.stoppedAt) {
            ctx.fillStyle = COLORS.matched;
            ctx.font = 'bold 10px "Segoe UI", system-ui, sans-serif';
            ctx.textAlign = 'right';
            ctx.fillText('UNTIL triggered \u2014 traversal stopped early', W - 8, 8);
        }
    }
}
function buildNoUntilSteps() {
    const steps = [];
    // Traverse the entire chain exhaustively
    for (let i = 0; i < chain.length; i++) {
        const nodeId = chain[i];
        steps.push(() => {
            state.traversedNodes.add(nodeId);
            state.currentNode = nodeId;
            if (i > 0) {
                state.traversedEdges.add(`${chain[i - 1]}->${nodeId}`);
            }
            const node = nodes.get(nodeId);
            if (i < chain.length - 1) {
                // Still going...
                const stepDesc = node.diedInOffice
                    ? `Visited ${node.label} (died in office!) \u2014 but traversal continues...`
                    : `Visited ${node.label} \u2014 continuing...`;
                if (state.steps.length <= i) {
                    state.steps.push({
                        description: `Step ${i + 1}: ${node.label} (${node.year})`,
                        detail: stepDesc,
                        active: true, done: false,
                    });
                }
                if (i > 0) {
                    state.steps[i - 1].active = false;
                    state.steps[i - 1].done = true;
                }
            }
            else {
                // Last node
                state.phase = 'done';
                state.currentNode = '';
                if (i > 0) {
                    state.steps[i - 1].active = false;
                    state.steps[i - 1].done = true;
                }
                state.steps.push({
                    description: `Without UNTIL: traversed all ${chain.length} nodes`,
                    detail: 'Standard SPARQL property paths have no way to stop early. Every node is visited even though we only needed the first who died in office.',
                    active: false, done: true,
                });
            }
            updateStepsUI();
        });
    }
    return steps;
}
function buildWithUntilSteps() {
    const steps = [];
    // Traverse until we find someone who died in office
    for (let i = 0; i < chain.length; i++) {
        const nodeId = chain[i];
        const node = nodes.get(nodeId);
        steps.push(() => {
            state.traversedNodes.add(nodeId);
            state.currentNode = nodeId;
            if (i > 0) {
                state.traversedEdges.add(`${chain[i - 1]}->${nodeId}`);
                state.steps[state.steps.length - 1].active = false;
                state.steps[state.steps.length - 1].done = true;
            }
            if (node.diedInOffice) {
                // EXIT!
                state.stoppedAt = nodeId;
                state.currentNode = '';
                state.phase = 'done';
                // Mark remaining as skipped
                for (let j = i + 1; j < chain.length; j++) {
                    state.skippedNodes.add(chain[j]);
                }
                state.steps.push({
                    description: `UNTIL triggered at ${node.label} (${node.year})!`,
                    detail: `Found first president who died in office after ${i + 1} hops. ${chain.length - i - 1} nodes skipped. Per-step predicate evaluation \u2014 no post-filtering needed.`,
                    active: false, done: true,
                });
            }
            else {
                state.steps.push({
                    description: `Step ${i + 1}: ${node.label} (${node.year})`,
                    detail: 'Checking exit condition: diedInOffice = false. Continue...',
                    active: true, done: false,
                });
            }
            updateStepsUI();
        });
        if (node.diedInOffice)
            break; // Don't generate steps past exit
    }
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
function runNoUntil() {
    resetState();
    state.phase = 'no-until';
    state.mode = 'no-until';
    state.stepQueue = buildNoUntilSteps();
    state.stepIndex = 0;
    state.running = true;
    playNext();
}
function runWithUntil() {
    resetState();
    state.phase = 'with-until';
    state.mode = 'with-until';
    state.stepQueue = buildWithUntilSteps();
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
        setTimeout(playNext, 400);
    }
}
function stepOnce() {
    if (state.phase === 'idle') {
        state.phase = 'with-until';
        state.mode = 'with-until';
        state.stepQueue = buildWithUntilSteps();
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
    document.getElementById('btn-no-until').addEventListener('click', runNoUntil);
    document.getElementById('btn-until').addEventListener('click', runWithUntil);
    document.getElementById('btn-reset').addEventListener('click', resetState);
    document.getElementById('btn-step').addEventListener('click', stepOnce);
    window.addEventListener('resize', resize);
    resize();
});
export {};
