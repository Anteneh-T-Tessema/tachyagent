/// Embedded web UI served from the daemon.
/// Single HTML file with everything inline — no build step, no npm, no webpack.
/// Modern, premium dark-mode interface with real-time performance analytics.

pub const INDEX_HTML: &str = r##"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>Tachy — AI Agent Platform</title>
    <link rel="preconnect" href="https://fonts.googleapis.com">
    <link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
    <link href="https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600;700&family=JetBrains+Mono:wght@400;500&display=swap" rel="stylesheet">
    <script src="https://cdn.jsdelivr.net/npm/chart.js"></script>
    <style>
        :root {
            --bg: #050506;
            --surface: rgba(22, 22, 26, 0.7);
            --surface-solid: #16161a;
            --border: rgba(255, 255, 255, 0.08);
            --text: #f4f4f5;
            --muted: #a1a1aa;
            --accent: #6366f1;
            --accent-glow: rgba(99, 102, 241, 0.4);
            --success: #10b981;
            --warning: #f59e0b;
            --error: #ef4444;
            --radius: 12px;
            --glass: blur(12px);
        }

        .thinking-block {
            font-size: 13px;
            color: var(--muted);
            font-style: italic;
            border-left: 2px solid var(--border);
            padding-left: 12px;
            margin-bottom: 12px;
            white-space: pre-wrap;
        }

        .text-content {
            white-space: pre-wrap;
        }

        * { margin: 0; padding: 0; box-sizing: border-box; }
        body { 
            font-family: 'Inter', system-ui, -apple-system, sans-serif; 
            background: var(--bg); 
            color: var(--text); 
            min-height: 100vh;
            overflow: hidden;
            background-image: 
                radial-gradient(circle at 0% 0%, rgba(99, 102, 241, 0.15) 0%, transparent 50%),
                radial-gradient(circle at 100% 100%, rgba(16, 185, 129, 0.05) 0%, transparent 50%);
        }

        /* Layout */
        .app { display: flex; height: 100vh; }
        
        .sidebar { 
            width: 280px; 
            background: var(--surface); 
            backdrop-filter: var(--glass);
            border-right: 1px solid var(--border); 
            padding: 24px; 
            display: flex; 
            flex-direction: column;
            z-index: 100;
        }

        .main { 
            flex: 1; 
            display: flex; 
            flex-direction: column; 
            overflow: hidden;
            position: relative;
        }

        .logo { 
            font-size: 24px; 
            font-weight: 700; 
            margin-bottom: 32px; 
            display: flex; 
            align-items: center; 
            gap: 12px;
            background: linear-gradient(135deg, #fff 0%, var(--accent) 100%);
            -webkit-background-clip: text;
            -webkit-text-fill-color: transparent;
        }

        .nav-section {
            font-size: 11px;
            font-weight: 600;
            text-transform: uppercase;
            color: var(--muted);
            margin: 24px 0 12px 12px;
            letter-spacing: 0.1em;
        }

        .nav-item { 
            padding: 12px 16px; 
            border-radius: var(--radius); 
            cursor: pointer; 
            margin-bottom: 4px; 
            color: var(--muted); 
            transition: all 0.2s cubic-bezier(0.4, 0, 0.2, 1);
            display: flex; 
            align-items: center; 
            gap: 12px;
            font-weight: 500;
        }

        .nav-item:hover { 
            background: rgba(255, 255, 255, 0.05); 
            color: var(--text);
            transform: translateX(4px);
        }

        .nav-item.active { 
            background: var(--accent); 
            color: white;
            box-shadow: 0 4px 15px var(--accent-glow);
        }

        /* Header */
        .header { 
            padding: 20px 32px; 
            background: rgba(5, 5, 6, 0.5);
            backdrop-filter: var(--glass);
            border-bottom: 1px solid var(--border); 
            display: flex; 
            align-items: center; 
            justify-content: space-between;
            z-index: 50;
        }

        .header h1 { font-size: 20px; font-weight: 600; letter-spacing: -0.02em; }

        .status-badge { 
            padding: 6px 12px; 
            border-radius: 20px; 
            font-size: 12px; 
            font-weight: 600; 
            display: flex;
            align-items: center;
            gap: 8px;
            border: 1px solid transparent;
        }
        .status-ok { background: rgba(16, 185, 129, 0.1); color: var(--success); border-color: rgba(16, 185, 129, 0.2); }
        .status-err { background: rgba(239, 68, 68, 0.1); color: var(--error); border-color: rgba(239, 68, 68, 0.2); }

        /* Content Areas */
        .content { 
            flex: 1; 
            overflow-y: auto; 
            padding: 32px; 
            display: none;
            animation: fadeIn 0.3s ease-out;
        }
        @keyframes fadeIn { from { opacity: 0; transform: translateY(10px); } to { opacity: 1; transform: translateY(0); } }

        .card { 
            background: var(--surface); 
            backdrop-filter: var(--glass);
            border: 1px solid var(--border); 
            border-radius: var(--radius); 
            padding: 24px; 
            margin-bottom: 24px;
            transition: border-color 0.2s;
        }
        .card:hover { border-color: rgba(255, 255, 255, 0.15); }
        .card h3 { font-size: 14px; font-weight: 600; margin-bottom: 20px; color: var(--muted); text-transform: uppercase; letter-spacing: 0.05em; }

        /* Grid */
        .stats-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(200px, 1fr)); gap: 20px; margin-bottom: 24px; }
        .stat-card { padding: 20px; background: var(--surface); border: 1px solid var(--border); border-radius: var(--radius); }
        .stat-value { font-size: 32px; font-weight: 700; color: var(--text); margin-bottom: 4px; }
        .stat-label { font-size: 12px; color: var(--muted); font-weight: 500; }

        /* Chat Specific */
        .chat-container { 
            height: 100%;
            display: flex; 
            flex-direction: column; 
            background: transparent;
        }
        .messages { 
            flex: 1; 
            overflow-y: auto; 
            padding: 32px; 
            scroll-behavior: smooth;
        }
        .message { margin-bottom: 24px; display: flex; flex-direction: column; max-width: 85%; }
        .message.user { margin-left: auto; align-items: flex-end; }
        .message.assistant { align-items: flex-start; }
        
        .bubble { 
            padding: 16px 20px; 
            line-height: 1.6; 
            border-radius: var(--radius);
            font-size: 15px;
            position: relative;
        }
        .message.user .bubble { background: var(--accent); color: white; border-bottom-right-radius: 4px; }
        .message.assistant .bubble { background: var(--surface-solid); border: 1px solid var(--border); border-bottom-left-radius: 4px; }
        
        .meta { font-size: 11px; color: var(--muted); margin-top: 8px; font-family: 'JetBrains Mono', monospace; }

        .input-bar { 
            padding: 24px 32px; 
            background: var(--bg);
            border-top: 1px solid var(--border); 
            display: flex; 
            gap: 12px; 
            align-items: center;
        }
        .input-bar input { 
            flex: 1; 
            background: var(--surface-solid); 
            border: 1px solid var(--border); 
            color: var(--text); 
            padding: 14px 20px; 
            border-radius: var(--radius); 
            font-size: 15px; 
            outline: none;
            transition: all 0.2s;
        }
        .input-bar input:focus { border-color: var(--accent); box-shadow: 0 0 0 4px rgba(99, 102, 241, 0.1); }
        
        .input-bar button { 
            background: var(--accent); 
            color: white; 
            border: none; 
            padding: 14px 24px; 
            border-radius: var(--radius); 
            cursor: pointer; 
            font-weight: 600;
            transition: all 0.2s;
        }
        .input-bar button:hover { transform: translateY(-1px); box-shadow: 0 4px 12px var(--accent-glow); }
        .input-bar button:disabled { opacity: 0.5; filter: grayscale(1); }

        /* Approvals */
        .approval-item { 
            display: flex; 
            justify-content: space-between; 
            align-items: center; 
            padding: 16px; 
            background: rgba(255, 255, 255, 0.03); 
            border-radius: var(--radius); 
            margin-bottom: 12px;
            border: 1px solid var(--border);
        }
        .btn-approve { background: var(--success); color: white; border: none; padding: 8px 16px; border-radius: 6px; cursor: pointer; font-weight: 600; }
        .btn-reject { background: var(--error); color: white; border: none; padding: 8px 16px; border-radius: 6px; cursor: pointer; font-weight: 600; }

        /* Tables & Lists */
        table { width: 100%; border-collapse: separate; border-spacing: 0 8px; }
        th { text-align: left; padding: 12px 16px; font-size: 12px; color: var(--muted); text-transform: uppercase; }
        td { padding: 16px; background: rgba(255, 255, 255, 0.02); }
        td:first-child { border-radius: var(--radius) 0 0 var(--radius); }
        td:last-child { border-radius: 0 var(--radius) var(--radius) 0; }
        
        code { font-family: 'JetBrains Mono', monospace; background: rgba(0,0,0,0.3); padding: 2px 6px; border-radius: 4px; color: var(--accent); }

        .spinner { 
            width: 18px; height: 18px; 
            border: 2.5px solid rgba(255,255,255,0.1); 
            border-top-color: currentColor; 
            border-radius: 50%; 
            animation: spin 0.8s linear infinite; 
        }
        @keyframes spin { to { transform: rotate(360deg); } }

        /* Scrollbars */
        ::-webkit-scrollbar { width: 6px; }
        ::-webkit-scrollbar-track { background: transparent; }
        ::-webkit-scrollbar-thumb { background: var(--border); border-radius: 10px; }
        ::-webkit-scrollbar-thumb:hover { background: var(--muted); }
    </style>
</head>
<body>
    <div class="app">
        <aside class="sidebar">
            <div class="logo">⚡ TACHY</div>
            
            <div class="nav-section">Core</div>
            <div class="nav-item active" onclick="showPage('chat', this)">💬 Conversations</div>
            <div class="nav-item" onclick="showPage('agents', this)">🤖 Agent Fleet</div>
            <div class="nav-item" onclick="showPage('approvals', this)">🛡️ Governance</div>
            
            <div class="nav-section">Performance</div>
            <div class="nav-item" onclick="showPage('dashboard', this)">📊 Metrics</div>
            <div class="nav-item" onclick="showPage('models', this)">🧠 Model Registry</div>
            
            <div class="nav-section">Audit</div>
            <div class="nav-item" onclick="showPage('audit', this)">📜 Audit Trail</div>
            <div class="nav-item" onclick="showPage('parallel', this)">⛓️ Parallel Ops</div>

            <div style="margin-top: auto; padding-top: 20px; border-top: 1px solid var(--border);">
                <div class="stat-label">Workspace Root</div>
                <div style="font-size: 11px; color: var(--text); opacity: 0.8; word-break: break-all; margin-top: 4px;" id="workspace-path">...</div>
            </div>
        </aside>

        <main class="main">
            <header class="header">
                <h1 id="page-title">Conversations</h1>
                <div id="status-container">
                    <span id="status-badge" class="status-badge status-ok">● Running</span>
                </div>
            </header>

            <!-- Pages -->
            <section id="page-chat" class="content chat-container" style="display: flex;">
                <div class="messages" id="messages">
                    <div class="message assistant">
                        <div class="bubble">Welcome back. I am Tachy, your localized intelligence engine. Ready for secure, token-driven development.</div>
                        <div class="meta">SYSTEM :: INITIALIZED</div>
                    </div>
                </div>
                <div class="input-bar">
                    <select id="model-select" style="background:var(--surface-solid); border:1px solid var(--border); color:var(--text); padding:10px; border-radius:var(--radius); outline:none;"></select>
                    <input type="text" id="chat-input" placeholder="Type a command or question..." onkeydown="if(event.key==='Enter')sendMessage()">
                    <button onclick="sendMessage()" id="send-btn">Transmit</button>
                    <button onclick="newConversation()" style="background:var(--surface-solid); color:var(--muted); padding:10px 14px;">+</button>
                </div>
            </section>

            <section id="page-dashboard" class="content">
                <div class="stats-grid">
                    <div class="stat-card">
                        <div class="stat-value" id="val-ttft">0ms</div>
                        <div class="stat-label">Last TTFT</div>
                    </div>
                    <div class="stat-card">
                        <div class="stat-value" id="val-p50">0ms</div>
                        <div class="stat-label">Median (P50)</div>
                    </div>
                    <div class="stat-card">
                        <div class="stat-value" id="val-p95">0ms</div>
                        <div class="stat-label">Tail Latency (P95)</div>
                    </div>
                    <div class="stat-card">
                        <div class="stat-value" id="val-tps">0.0</div>
                        <div class="stat-label">Throughput /s</div>
                    </div>
                    <div class="stat-card">
                        <div class="stat-value" id="val-total-tokens">0</div>
                        <div class="stat-label">Total Tokens</div>
                    </div>
                    <div class="stat-card">
                        <div class="stat-value" id="val-requests">0</div>
                        <div class="stat-label">Total Inferences</div>
                    </div>
                </div>
                
                <div class="stats-grid" style="grid-template-columns: 1fr 1fr;">
                    <div class="card" style="height: 400px;">
                        <h3>Inference Latency (TTFT)</h3>
                        <canvas id="ttftChart"></canvas>
                    </div>
                    <div class="card" style="height: 400px;">
                        <h3>Throughput (Tokens/Sec)</h3>
                        <canvas id="tpsChart"></canvas>
                    </div>
                </div>
            </section>

            <section id="page-approvals" class="content">
                <div class="card">
                    <h3>Pending Security Review</h3>
                    <div id="approvals-list"></div>
                </div>
                <div class="card">
                    <h3>Active File Locks</h3>
                    <table id="locks-table-body"></table>
                </div>
            </section>

            <section id="page-agents" class="content">
                <div class="card">
                    <h3>Deploy Agent Instance</h3>
                    <div style="display:flex; gap:12px; margin-top:16px;">
                        <input type="text" id="agent-prompt" placeholder="Describe the mission..." style="flex:1; background:var(--surface-solid); border:1px solid var(--border); color:var(--text); padding:12px; border-radius:var(--radius);">
                        <select id="agent-template" style="background:var(--surface-solid); border:1px solid var(--border); color:var(--text); padding:12px; border-radius:var(--radius);"></select>
                        <button onclick="runAgent()" id="run-agent-btn" style="background:var(--accent); color:white; border:none; padding:12px 24px; border-radius:var(--radius); cursor:pointer;">Launch</button>
                    </div>
                </div>
                <div class="card">
                    <h3>Fleet Status</h3>
                    <table id="agents-table-body"></table>
                </div>
            </section>

            <section id="page-models" class="content">
                <div class="card">
                    <h3>Model Inventory</h3>
                    <table id="models-table-body"></table>
                </div>
            </section>

            <section id="page-audit" class="content">
                <div class="card">
                    <h3>System Audit Log</h3>
                    <table id="audit-table-body"></table>
                </div>
            </section>
            
            <section id="page-parallel" class="content">
                <div class="card">
                    <h3>Parallel Execution Streams</h3>
                    <table id="parallel-table-body"></table>
                </div>
            </section>
        </main>
    </div>

    <script>
        const API = '';
        let currentConvId = localStorage.getItem('tachy_current_conv') || '';
        let ttftHistory = [];
        let tpsHistory = [];
        let ttftChart, tpsChart;

        async function apiFetch(path, opts = {}) {
            const apiKey = localStorage.getItem('tachy_api_key') || '';
            const headers = { 'Content-Type': 'application/json', ...opts.headers };
            if (apiKey) headers['Authorization'] = `Bearer ${apiKey}`;
            return fetch(API + path, { ...opts, headers });
        }

        function showPage(page, navEl) {
            document.querySelectorAll('.nav-item').forEach(el => el.classList.remove('active'));
            if (navEl) navEl.classList.add('active');
            
            document.querySelectorAll('.content').forEach(el => el.style.display = 'none');
            const target = document.getElementById('page-' + page);
            if (target) target.style.display = page === 'chat' ? 'flex' : 'block';
            
            document.getElementById('page-title').textContent = page.charAt(0).toUpperCase() + page.slice(1);
            
            if (page === 'dashboard') initCharts();
            refreshPageData(page);
        }

        async function refreshPageData(page) {
            switch(page) {
                case 'dashboard': loadMetrics(); break;
                case 'approvals': loadApprovals(); loadLocks(); break;
                case 'agents': loadAgents(); break;
                case 'models': loadModels(); break;
                case 'audit': loadAudit(); break;
                case 'parallel': loadParallel(); break;
            }
        }

        // Charts
        function initCharts() {
            if (ttftChart) return;
            const commonOptions = {
                responsive: true,
                maintainAspectRatio: false,
                scales: { 
                    y: { grid: { color: 'rgba(255,255,255,0.05)' }, ticks: { color: '#a1a1aa' } },
                    x: { grid: { display: false }, ticks: { display: false } }
                },
                plugins: { legend: { display: false } },
                elements: { line: { tension: 0.4 }, point: { radius: 0 } }
            };

            ttftChart = new Chart(document.getElementById('ttftChart'), {
                type: 'line',
                data: { labels: [], datasets: [{ data: [], borderColor: '#6366f1', borderWidth: 2, fill: true, backgroundColor: 'rgba(99,102,241,0.1)' }] },
                options: commonOptions
            });

            tpsChart = new Chart(document.getElementById('tpsChart'), {
                type: 'line',
                data: { labels: [], datasets: [{ data: [], borderColor: '#10b981', borderWidth: 2, fill: true, backgroundColor: 'rgba(16, 185, 129,0.1)' }] },
                options: commonOptions
            });
        }

        async function loadMetrics() {
            try {
                const r = await apiFetch('/api/inference/stats');
                const stats = await r.json();
                
                document.getElementById('val-ttft').textContent = `${stats.last_ttft_ms}ms`;
                document.getElementById('val-p50').textContent = `${stats.p50_ttft_ms.toFixed(0)}ms`;
                document.getElementById('val-p95').textContent = `${stats.p95_ttft_ms.toFixed(0)}ms`;
                document.getElementById('val-tps').textContent = stats.last_tokens_per_sec.toFixed(1);
                document.getElementById('val-total-tokens').textContent = stats.total_tokens.toLocaleString();
                document.getElementById('val-requests').textContent = stats.total_requests.toLocaleString();

                // Mocking some history if empty just for the "wow" factor, otherwise update real
                if (ttftChart) {
                    ttftChart.data.labels.push('');
                    ttftChart.data.datasets[0].data.push(stats.last_ttft_ms);
                    if (ttftChart.data.labels.length > 20) { ttftChart.data.labels.shift(); ttftChart.data.datasets[0].data.shift(); }
                    ttftChart.update();

                    tpsChart.data.labels.push('');
                    tpsChart.data.datasets[0].data.push(stats.last_tokens_per_sec);
                    if (tpsChart.data.labels.length > 20) { tpsChart.data.labels.shift(); tpsChart.data.datasets[0].data.shift(); }
                    tpsChart.update();
                }
            } catch(e) {}
        }

        // Approvals
        async function loadApprovals() {
            const r = await apiFetch('/api/pending-approvals');
            const data = await r.json();
            const list = document.getElementById('approvals-list');
            const items = data.pending || [];
            
            if (items.length === 0) {
                list.innerHTML = '<p style="color:var(--muted); font-size: 13px;">No pending actions requiring authorization.</p>';
                return;
            }

            list.innerHTML = items.map(p => `
                <div class="approval-item">
                    <div>
                        <div style="font-weight:600; margin-bottom:4px;">${p.patch.file_path}</div>
                        <div style="font-size:12px; color:var(--muted); font-family: 'JetBrains Mono';">${p.reason}</div>
                        <div style="font-size:11px; margin-top:4px;">ID: ${p.id} · Agent: ${p.patch.agent_id}</div>
                    </div>
                    <div style="display:flex; gap:8px;">
                        <button class="btn-approve" onclick="decidePatch('${p.id}', true)">Approve</button>
                        <button class="btn-reject" onclick="decidePatch('${p.id}', false)">Deny</button>
                    </div>
                </div>
            `).join('');
        }

        async function decidePatch(id, approved) {
            await apiFetch('/api/approve', {
                method: 'POST',
                body: JSON.stringify({ patch_id: id, approved })
            });
            loadApprovals();
        }

        // Chat & Streaming
        async function sendMessage() {
            const input = document.getElementById('chat-input');
            const prompt = input.value.trim();
            if (!prompt) return;
            input.value = '';

            addMessage('user', prompt);
            
            const btn = document.getElementById('send-btn');
            btn.disabled = true;
            btn.innerHTML = '<span class="spinner"></span>';

            const assistantMsgId = 'msg-' + Date.now();
            addMessage('assistant', '', assistantMsgId);
            const bubble = document.getElementById(assistantMsgId).querySelector('.bubble');
            let content = '';

            try {
                const model = document.getElementById('model-select').value;
                const response = await apiFetch('/api/chat/stream', {
                    method: 'POST',
                    body: JSON.stringify({ prompt, model })
                });

                const reader = response.body.getReader();
                const decoder = new TextDecoder();

                while (true) {
                    const { done, value } = await reader.read();
                    if (done) break;

                    const chunk = decoder.decode(value, { stream: true });
                    const lines = chunk.split('\n');
                    
                    for (const line of lines) {
                        if (line.startsWith('data: ')) {
                            try {
                                const data = JSON.parse(line.slice(6));
                                if (data.text) {
                                    content += data.text;
                                    bubble.querySelector('.text-content').textContent = content;
                                } else if (data.thinking) {
                                    let thinkEl = bubble.querySelector('.thinking-block');
                                    if (!thinkEl) {
                                        thinkEl = document.createElement('div');
                                        thinkEl.className = 'thinking-block';
                                        bubble.insertBefore(thinkEl, bubble.firstChild);
                                    }
                                    thinkEl.textContent += data.thinking;
                                }
                            } catch(e) {}
                        }
                    }
                }
            } catch(e) {
                bubble.textContent = 'Error: Link to daemon severed.';
            }

            btn.disabled = false;
            btn.textContent = 'Transmit';
        }

        function addMessage(role, text, id) {
            const div = document.createElement('div');
            div.className = `message ${role}`;
            if (id) div.id = id;
            div.innerHTML = `
                <div class="bubble">
                    <div class="text-content">${text || (role==='assistant'?'<span class="spinner"></span>':'')}</div>
                </div>
                <div class="meta">${role.toUpperCase()} :: ${new Date().toLocaleTimeString()}</div>
            `;
            const container = document.getElementById('messages');
            container.appendChild(div);
            container.scrollTop = container.scrollHeight;
        }

        // Generic Loaders
        async function loadModels() {
            const r = await apiFetch('/api/models');
            const models = await r.json();
            const select = document.getElementById('model-select');
            const tbody = document.getElementById('models-table-body');
            
            select.innerHTML = models.map(m => `<option value="${m.name}">${m.name}</option>`).join('');
            tbody.innerHTML = '<thead><tr><th>Model</th><th>Backend</th><th>Context</th><th>Tools</th></tr></thead>' + 
                models.map(m => `<tr><td>${m.name}</td><td>${m.backend}</td><td>${m.context_window}</td><td>${m.supports_tool_use?'✓':'-'}</td></tr>`).join('');
        }

        async function loadAgents() {
            const r = await apiFetch('/api/agents');
            const agents = await r.json();
            const tbody = document.getElementById('agents-table-body');
            tbody.innerHTML = '<thead><tr><th>ID</th><th>Template</th><th>Status</th><th>Ops</th></tr></thead>' + 
                agents.map(a => `<tr><td><code>${a.id}</code></td><td>${a.template}</td><td>${a.status}</td><td>${a.tool_invocations}</td></tr>`).join('');
        }

        async function loadAudit() {
            const r = await apiFetch('/api/audit');
            const audit = await r.json();
            const tbody = document.getElementById('audit-table-body');
            tbody.innerHTML = '<thead><tr><th>Timestamp</th><th>Kind</th><th>Detail</th></tr></thead>' + 
                audit.slice(-20).reverse().map(e => `<tr><td style="font-size:11px;">${e.timestamp}</td><td>${e.kind}</td><td style="font-size:12px;">${e.detail}</td></tr>`).join('');
        }

        async function loadParallel() {
            const r = await apiFetch('/api/parallel/runs');
            const data = await r.json();
            const tbody = document.getElementById('parallel-table-body');
            const runs = data.runs || [];
            tbody.innerHTML = '<thead><tr><th>Run ID</th><th>Status</th><th>Tasks</th></tr></thead>' + 
                runs.map(r => `<tr><td><code>${r.run_id}</code></td><td>${r.status}</td><td>${r.task_count}</td></tr>`).join('');
        }

        async function loadLocks() {
            const r = await apiFetch('/api/file-locks');
            const data = await r.json();
            const tbody = document.getElementById('locks-table-body');
            const locks = data.locks || [];
            tbody.innerHTML = '<thead><tr><th>File Path</th><th>Agent Holder</th></tr></thead>' + 
                (locks.length ? locks.map(l => `<tr><td>${l.file}</td><td><code>${l.agent_id}</code></td></tr>`).join('') : '<tr><td colspan="2" style="color:var(--muted)">No active locks.</td></tr>');
        }

        async function loadTemplates() {
            const r = await apiFetch('/api/templates');
            const templates = await r.json();
            const select = document.getElementById('agent-template');
            select.innerHTML = templates.map(t => `<option value="${t.name}">${t.name} (${t.model})</option>`).join('');
        }

        async function runAgent() {
            const prompt = document.getElementById('agent-prompt').value.trim();
            const template = document.getElementById('agent-template').value;
            if (!prompt) return;

            const btn = document.getElementById('run-agent-btn');
            btn.disabled = true;
            btn.innerHTML = '<span class="spinner"></span>';

            try {
                const r = await apiFetch('/api/agents/run', {
                    method: 'POST',
                    body: JSON.stringify({ template, prompt })
                });
                const res = await r.json();
                if (res.agent_id) {
                    showPage('agents', document.querySelector('[onclick*="agents"]'));
                    loadAgents();
                    document.getElementById('agent-prompt').value = '';
                }
            } catch(e) {} finally {
                btn.disabled = false;
                btn.textContent = 'Launch';
            }
        }

        async function checkHealth() {
            try {
                const r = await apiFetch('/health');
                const d = await r.json();
                document.getElementById('status-badge').className = 'status-badge status-ok';
                document.getElementById('status-badge').textContent = '● Online';
            } catch(e) {
                document.getElementById('status-badge').className = 'status-badge status-err';
                document.getElementById('status-badge').textContent = '● Offline';
            }
        }

        // Initialize
        (async () => {
            const health = await (await apiFetch('/health')).json();
            document.getElementById('workspace-path').textContent = health.workspace || 'Local';
            loadModels();
            loadTemplates();
            checkHealth();
            setInterval(checkHealth, 5000);
            setInterval(() => { if(ttftChart) loadMetrics(); }, 3000);
        })();
    </script>
</body>
</html>
"##;
