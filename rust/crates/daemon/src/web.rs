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
            <div class="nav-item" onclick="showPage('search', this)">🔍 Code Search</div>
            
            <div class="nav-section">Performance</div>
            <div class="nav-item" onclick="showPage('dashboard', this)">📊 Metrics</div>
            <div class="nav-item" onclick="showPage('models', this)">🧠 Model Registry</div>
            
            <div class="nav-section">Audit</div>
            <div class="nav-item" onclick="showPage('audit', this)">📜 Audit Trail</div>
            <div class="nav-item" onclick="showPage('parallel', this)">⛓️ Parallel Ops</div>
            
            <div class="nav-section">Enterprise Scaling</div>
            <div class="nav-item" onclick="showPage('mission', this)">📡 Mission Control</div>
            <div class="nav-item" onclick="showPage('swarm', this)">🐝 Agent Swarm</div>
            <div class="nav-item" onclick="showPage('marketplace', this)">🏪 Marketplace</div>
            <div class="nav-item" onclick="showPage('cloud', this)">☁️ AWS Batch</div>
            <div class="nav-item" onclick="showPage('identity', this)">🛡️ Enterprise Identity</div>
            <div class="nav-item" onclick="showPage('usage', this)">📈 Usage Metering</div>
            <div class="nav-item" onclick="showPage('finetune', this)">🔬 Fine-tuning</div>
            <div class="nav-item" onclick="showPage('depgraph', this)">🕸️ Dep Graph</div>
            <div class="nav-item" onclick="showPage('events', this)">⚡ Live Events</div>
            <div class="nav-item" onclick="showPage('runtemplates', this)">📋 DAG Templates</div>

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
                    <div class="stat-card">
                        <div class="stat-value" id="val-workers">0</div>
                        <div class="stat-label">Active Swarm Workers</div>
                    </div>
                    <div class="stat-card">
                        <div class="stat-value" id="val-cloud">Off</div>
                        <div class="stat-label">Cloud Sync</div>
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

                <!-- E2: Cost estimate + model leaderboard -->
                <div class="stats-grid" style="grid-template-columns: 1fr 2fr; margin-top: 16px;">
                    <div class="card">
                        <h3>Cost Estimate (E2)</h3>
                        <div style="margin-top: 14px;">
                            <div style="font-size: 32px; font-weight: 700; color: var(--success);" id="val-cost-usd">$0.0000</div>
                            <div style="font-size: 12px; color: var(--muted); margin-top: 4px;">local compute proxy · $0.002 / 1k tokens</div>
                        </div>
                        <div style="margin-top: 16px; padding-top: 16px; border-top: 1px solid var(--border); font-size: 13px; color: var(--muted);">
                            <div style="display:flex; justify-content:space-between; margin-bottom:6px;">
                                <span>Input tokens</span><span id="val-input-tokens" style="color:var(--text);">0</span>
                            </div>
                            <div style="display:flex; justify-content:space-between;">
                                <span>Output tokens</span><span id="val-output-tokens" style="color:var(--text);">0</span>
                            </div>
                        </div>
                    </div>
                    <div class="card">
                        <h3>Model Leaderboard</h3>
                        <div style="margin-top: 14px; overflow-x: auto;">
                            <table style="width:100%; border-collapse:collapse; font-size:13px;">
                                <thead>
                                    <tr style="color:var(--muted); text-align:left;">
                                        <th style="padding:6px 0; font-weight:500;">#</th>
                                        <th style="padding:6px 8px; font-weight:500;">Model</th>
                                        <th style="padding:6px 8px; font-weight:500; text-align:right;">Tokens</th>
                                        <th style="padding:6px 8px; font-weight:500; text-align:right;">Avg tok/s</th>
                                        <th style="padding:6px 8px; font-weight:500; text-align:right;">P50 TTFT</th>
                                        <th style="padding:6px 8px; font-weight:500; text-align:right;">Tier</th>
                                    </tr>
                                </thead>
                                <tbody id="model-leaderboard">
                                    <tr><td colspan="6" style="color:var(--muted); padding:10px 0; font-size:12px;">Loading…</td></tr>
                                </tbody>
                            </table>
                        </div>
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
                <div class="card">
                    <h3>Side-by-Side Model Comparison</h3>
                    <div style="display:grid; gap:12px; margin-top:16px;">
                        <textarea id="cmp-prompt" rows="3" placeholder="Enter a prompt to run on both models…"
                            style="background:var(--surface-solid); border:1px solid var(--border); color:var(--text); padding:12px; border-radius:var(--radius); resize:vertical; font-family:monospace; font-size:12px;"></textarea>
                        <div style="display:grid; grid-template-columns:1fr 1fr; gap:12px;">
                            <input type="text" id="cmp-model-a" placeholder="Model A (e.g. gemma4:26b)"
                                style="background:var(--surface-solid); border:1px solid var(--border); color:var(--text); padding:10px; border-radius:var(--radius); font-family:monospace; font-size:12px;">
                            <input type="text" id="cmp-model-b" placeholder="Model B (e.g. llama3.3)"
                                style="background:var(--surface-solid); border:1px solid var(--border); color:var(--text); padding:10px; border-radius:var(--radius); font-family:monospace; font-size:12px;">
                        </div>
                        <div style="display:flex; gap:10px; align-items:center;">
                            <button onclick="runComparison()" style="background:var(--accent); color:white; border:none; padding:12px 24px; border-radius:var(--radius); cursor:pointer; font-weight:600;">Compare</button>
                            <span id="cmp-status" style="font-size:13px; color:var(--muted);"></span>
                        </div>
                    </div>
                    <div id="cmp-results" style="display:none; margin-top:20px;">
                        <div style="display:grid; grid-template-columns:1fr 1fr; gap:16px;">
                            <div>
                                <div id="cmp-header-a" style="font-weight:600; margin-bottom:8px; color:var(--accent);"></div>
                                <div id="cmp-meta-a" style="font-size:11px; color:var(--muted); margin-bottom:8px;"></div>
                                <pre id="cmp-output-a" style="background:rgba(0,0,0,0.3); padding:12px; border-radius:8px; font-size:12px; white-space:pre-wrap; word-break:break-word; max-height:400px; overflow-y:auto;"></pre>
                            </div>
                            <div>
                                <div id="cmp-header-b" style="font-weight:600; margin-bottom:8px; color:#a78bfa;"></div>
                                <div id="cmp-meta-b" style="font-size:11px; color:var(--muted); margin-bottom:8px;"></div>
                                <pre id="cmp-output-b" style="background:rgba(0,0,0,0.3); padding:12px; border-radius:8px; font-size:12px; white-space:pre-wrap; word-break:break-word; max-height:400px; overflow-y:auto;"></pre>
                            </div>
                        </div>
                    </div>
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

            <section id="page-swarm" class="content">
                <div class="card">
                    <h3>Launch Swarm Run</h3>
                    <div style="display:grid; gap:12px; margin-top:16px;">
                        <input type="text" id="swarm-goal" placeholder="Describe the goal (e.g. Add structured logging to all HTTP handlers)" style="background:var(--surface-solid); border:1px solid var(--border); color:var(--text); padding:12px; border-radius:var(--radius);">
                        <textarea id="swarm-files" rows="3" placeholder="Workspace-relative file paths, one per line (leave blank to auto-discover)" style="background:var(--surface-solid); border:1px solid var(--border); color:var(--text); padding:12px; border-radius:var(--radius); resize:vertical; font-family:monospace; font-size:12px;"></textarea>
                        <div style="display:flex; gap:10px; align-items:center;">
                            <button onclick="launchSwarm()" style="background:var(--accent); color:white; border:none; padding:12px 24px; border-radius:var(--radius); cursor:pointer; font-weight:600;">Launch Swarm</button>
                            <span id="swarm-launch-status" style="font-size:13px; color:var(--muted);"></span>
                        </div>
                    </div>
                </div>
                <div class="card">
                    <h3>Active Swarm Clusters</h3>
                    <div id="swarm-list"></div>
                </div>
                <div class="card">
                    <h3>Task DAG Visualizer</h3>
                    <div id="swarm-dag-container" style="min-height: 300px; background: rgba(0,0,0,0.2); border-radius: 8px; padding: 16px; position: relative;">
                        <div id="swarm-dag-hint" style="color:var(--muted); font-size:12px; text-align:center; padding:40px 0;">
                            Click a swarm run above to visualize its task DAG.
                        </div>
                        <svg id="swarm-dag-svg" style="width:100%; display:none;" xmlns="http://www.w3.org/2000/svg"></svg>
                    </div>
                </div>
            </section>

            <section id="page-cloud" class="content">
                <div class="card">
                    <h3>Submit Cloud Job</h3>
                    <div style="display:grid; gap:12px; margin-top:16px;">
                        <input type="text" id="cloud-job-name" placeholder="Job name (e.g. refactor-auth-module)" style="background:var(--surface-solid); border:1px solid var(--border); color:var(--text); padding:12px; border-radius:var(--radius);">
                        <input type="text" id="cloud-job-cmd" placeholder="Command (e.g. tachy run --goal 'add logging')" style="background:var(--surface-solid); border:1px solid var(--border); color:var(--text); padding:12px; border-radius:var(--radius); font-family:monospace; font-size:12px;">
                        <div style="display:flex; gap:10px; align-items:center;">
                            <button onclick="submitCloudJob()" style="background:var(--accent); color:white; border:none; padding:12px 24px; border-radius:var(--radius); cursor:pointer; font-weight:600;">Submit to AWS Batch</button>
                            <span id="cloud-submit-status" style="font-size:13px; color:var(--muted);"></span>
                        </div>
                    </div>
                </div>
                <div class="card">
                    <h3>AWS Batch Dashboard</h3>
                    <div class="stats-grid">
                        <div class="stat-card">
                            <div class="stat-value">Active</div>
                            <div class="stat-label">Compute Environment</div>
                        </div>
                        <div class="stat-card">
                            <div class="stat-value" id="val-cloud-jobs">0</div>
                            <div class="stat-label">Jobs in Queue</div>
                        </div>
                    </div>
                    <table id="cloud-table-body"></table>
                </div>
            </section>

            <section id="page-mission" class="content">
                <div class="card">
                    <h3>Swarm Mission Feed</h3>
                    <div id="mission-feed-list" style="max-height: 500px; overflow-y: auto;"></div>
                </div>
            </section>

            <section id="page-marketplace" class="content">
                <div class="card">
                    <h3>Public Marketplace</h3>
                    <div id="marketplace-public" class="grid" style="display:grid; grid-template-columns: repeat(auto-fit, minmax(250px, 1fr)); gap:16px;"></div>
                </div>
                <div class="card">
                    <h3>Team Collection</h3>
                    <div id="marketplace-team" class="grid" style="display:grid; grid-template-columns: repeat(auto-fit, minmax(250px, 1fr)); gap:16px;"></div>
                </div>
            </section>

            <section id="page-identity" class="content">
                <div class="card">
                    <h3>SAML 2.0 / OIDC Configuration</h3>
                    <div style="display:grid; gap:16px; margin-top:16px;">
                        <input type="text" id="sso-idp-url" placeholder="IdP SSO URL" style="background:var(--surface-solid); border:1px solid var(--border); color:var(--text); padding:12px; border-radius:var(--radius);">
                        <input type="text" id="sso-sp-entity" placeholder="SP Entity ID (e.g. tachy-dev)" style="background:var(--surface-solid); border:1px solid var(--border); color:var(--text); padding:12px; border-radius:var(--radius);">
                        <button onclick="saveSsoConfig()" style="background:var(--accent); color:white; border:none; padding:12px; border-radius:var(--radius); cursor:pointer;">Update Identity Provider</button>
                    </div>
                </div>
            </section>

            <section id="page-usage" class="content">
                <div class="card">
                    <h3>Usage Metering</h3>
                    <div class="stats-grid" style="margin-top:16px;">
                        <div class="stat-card"><div class="stat-value" id="usage-total-tokens">—</div><div class="stat-label">Total Tokens</div></div>
                        <div class="stat-card"><div class="stat-value" id="usage-total-tools">—</div><div class="stat-label">Tool Invocations</div></div>
                        <div class="stat-card"><div class="stat-value" id="usage-total-runs">—</div><div class="stat-label">Agent Runs</div></div>
                    </div>
                </div>
                <div class="card">
                    <h3>Per-User Breakdown</h3>
                    <table id="usage-table"></table>
                </div>
            </section>

            <section id="page-finetune" class="content">
                <div class="card">
                    <h3>Extract Fine-tuning Dataset</h3>
                    <p style="font-size:13px; color:var(--muted); margin-bottom:16px;">
                        Converts your Tachy session history into Alpaca-format JSONL for LoRA fine-tuning.
                    </p>
                    <div style="display:flex; gap:10px; align-items:center;">
                        <button onclick="extractDataset()" style="background:var(--accent); color:white; border:none; padding:12px 24px; border-radius:var(--radius); cursor:pointer; font-weight:600;">Extract Dataset</button>
                        <span id="finetune-status" style="font-size:13px; color:var(--muted);"></span>
                    </div>
                    <div id="finetune-dataset-info" style="margin-top:16px; display:none;">
                        <div style="font-size:13px; color:var(--muted); margin-bottom:8px;" id="finetune-stats"></div>
                        <button onclick="downloadJsonl()" style="background:rgba(255,255,255,0.06); border:1px solid var(--border); color:var(--text); padding:8px 16px; border-radius:var(--radius); font-size:12px; cursor:pointer;">⬇ Download JSONL</button>
                    </div>
                </div>
                <div class="card">
                    <h3>Generate Ollama Modelfile</h3>
                    <div style="display:grid; gap:12px; margin-top:16px;">
                        <input type="text" id="ft-base-model" placeholder="Base model (e.g. mistral:7b)" value="gemma4:27b"
                            style="background:var(--surface-solid); border:1px solid var(--border); color:var(--text); padding:10px; border-radius:var(--radius); font-family:monospace; font-size:12px;">
                        <input type="text" id="ft-adapter-path" placeholder="LoRA adapter path (e.g. ./adapter.gguf)"
                            style="background:var(--surface-solid); border:1px solid var(--border); color:var(--text); padding:10px; border-radius:var(--radius); font-family:monospace; font-size:12px;">
                        <textarea id="ft-system-prompt" rows="2" placeholder="Custom system prompt (optional)"
                            style="background:var(--surface-solid); border:1px solid var(--border); color:var(--text); padding:10px; border-radius:var(--radius); resize:vertical; font-family:monospace; font-size:12px;"></textarea>
                        <div style="display:flex; gap:10px; align-items:center;">
                            <button onclick="generateModelfile()" style="background:var(--accent); color:white; border:none; padding:10px 20px; border-radius:var(--radius); cursor:pointer; font-weight:600;">Generate Modelfile</button>
                            <span id="modelfile-status" style="font-size:13px; color:var(--muted);"></span>
                        </div>
                        <pre id="modelfile-output" style="display:none; background:rgba(0,0,0,0.4); padding:12px; border-radius:8px; font-size:11px; white-space:pre-wrap; max-height:300px; overflow-y:auto;"></pre>
                    </div>
                </div>
            </section>

            <section id="page-depgraph" class="content">
                <div class="card">
                    <h3>Dependency Graph Explorer</h3>
                    <div style="display:flex; gap:10px; margin-top:16px;">
                        <input type="text" id="depgraph-file" placeholder="File path (e.g. src/main.rs)"
                            style="flex:1; background:var(--surface-solid); border:1px solid var(--border); color:var(--text); padding:12px; border-radius:var(--radius); font-family:monospace; font-size:12px;"
                            onkeydown="if(event.key==='Enter') loadDepGraph();">
                        <button onclick="loadDepGraph()" style="background:var(--accent); color:white; border:none; padding:12px 20px; border-radius:var(--radius); cursor:pointer;">Analyze</button>
                        <button onclick="loadFullGraph()" style="background:rgba(255,255,255,0.06); border:1px solid var(--border); color:var(--text); padding:12px 16px; border-radius:var(--radius); cursor:pointer; font-size:12px;">Full Graph</button>
                    </div>
                    <div id="depgraph-summary" style="display:none; margin-top:16px;">
                        <div style="display:grid; grid-template-columns: 1fr 1fr 1fr; gap:12px; margin-bottom:16px;">
                            <div class="stat-card"><div class="stat-value" id="dg-imports-count">0</div><div class="stat-label">Direct Imports</div></div>
                            <div class="stat-card"><div class="stat-value" id="dg-imported-by-count">0</div><div class="stat-label">Imported By</div></div>
                            <div class="stat-card"><div class="stat-value" id="dg-transitive-count">0</div><div class="stat-label">Transitive Dependents</div></div>
                        </div>
                    </div>
                </div>
                <div class="card" id="depgraph-vis-card" style="display:none;">
                    <h3 id="depgraph-vis-title">Graph</h3>
                    <svg id="depgraph-svg" style="width:100%; min-height:320px;" xmlns="http://www.w3.org/2000/svg"></svg>
                </div>
                <div class="card" id="depgraph-full-card" style="display:none;">
                    <h3>All Files — <span id="dg-total-nodes">0</span> nodes, <span id="dg-total-edges">0</span> edges</h3>
                    <div id="depgraph-file-list" style="max-height:400px; overflow-y:auto; font-size:12px; font-family:monospace;"></div>
                </div>
            </section>

            <!-- Wave 2A: Live SSE event stream -->
            <section id="page-events" class="content">
                <div class="card" style="display:flex; justify-content:space-between; align-items:center; padding-bottom:12px; border-bottom:1px solid var(--border);">
                    <div>
                        <h3 style="margin:0;">⚡ Live Event Stream</h3>
                        <div style="font-size:12px; color:var(--muted); margin-top:4px;">Real-time SSE feed from the daemon event bus — no polling.</div>
                    </div>
                    <div style="display:flex; gap:10px; align-items:center;">
                        <span id="sse-status-badge" style="font-size:11px; padding:4px 10px; border-radius:20px; background:rgba(239,68,68,0.15); color:var(--error);">● Disconnected</span>
                        <button onclick="clearEvents()" style="background:rgba(255,255,255,0.06); border:1px solid var(--border); color:var(--text); padding:6px 12px; border-radius:6px; font-size:12px; cursor:pointer;">Clear</button>
                    </div>
                </div>
                <div id="event-feed" style="font-family:monospace; font-size:12px; line-height:1.7; max-height:calc(100vh - 220px); overflow-y:auto; margin-top:12px; padding:0 4px;">
                    <div style="color:var(--muted); text-align:center; padding:40px 0;">Waiting for events…</div>
                </div>
            </section>

            <!-- Wave 2D: Named DAG templates -->
            <section id="page-runtemplates" class="content">
                <div class="card">
                    <h3>New DAG Template</h3>
                    <div style="display:grid; gap:12px; margin-top:16px;">
                        <input type="text" id="tpl-name" placeholder="Template name (e.g. refactor-auth)" style="background:var(--surface-solid); border:1px solid var(--border); color:var(--text); padding:12px; border-radius:var(--radius); font-family:monospace; font-size:13px;">
                        <textarea id="tpl-tasks" rows="6" placeholder='JSON tasks array — e.g.\n[{"template":"chat","prompt":"Refactor auth module"},{"template":"code-reviewer","prompt":"Review changes","deps":["refactor-auth"]}]' style="background:var(--surface-solid); border:1px solid var(--border); color:var(--text); padding:12px; border-radius:var(--radius); resize:vertical; font-family:monospace; font-size:12px;"></textarea>
                        <div style="display:flex; gap:10px; align-items:center;">
                            <button onclick="saveRunTemplate()" style="background:var(--accent); color:white; border:none; padding:12px 24px; border-radius:var(--radius); cursor:pointer; font-weight:600;">Save Template</button>
                            <span id="tpl-save-status" style="font-size:13px; color:var(--muted);"></span>
                        </div>
                    </div>
                </div>
                <div class="card">
                    <h3>Saved Templates</h3>
                    <table id="runtemplates-table" style="width:100%; margin-top:12px; border-collapse:collapse;"></table>
                </div>
            </section>

            <section id="page-search" class="content">
                <div class="card">
                    <div style="display:flex; justify-content:space-between; align-items:center;">
                        <h3>Semantic Code Search (C2)</h3>
                        <div style="display:flex; align-items:center; gap:10px;">
                            <span id="index-status" style="font-size:12px; color:var(--muted);">Index: checking…</span>
                            <button id="build-index-btn" onclick="buildIndex()" style="background:rgba(255,255,255,0.06); border:1px solid var(--border); color:var(--text); padding:6px 12px; border-radius:6px; font-size:12px; cursor:pointer;">Build Index</button>
                        </div>
                    </div>
                    <div style="display:flex; gap:10px; margin-top:16px;">
                        <input type="text" id="search-query" placeholder="Search functions, types, symbols, concepts…" style="flex:1; background:var(--surface-solid); border:1px solid var(--border); color:var(--text); padding:12px; border-radius:var(--radius);" onkeydown="if(event.key==='Enter') runCodeSearch();">
                        <select id="search-limit" style="background:var(--surface-solid); border:1px solid var(--border); color:var(--text); padding:12px; border-radius:var(--radius);">
                            <option value="10">10</option>
                            <option value="25">25</option>
                            <option value="50">50</option>
                        </select>
                        <button onclick="runCodeSearch()" style="background:var(--accent); color:white; border:none; padding:12px 20px; border-radius:var(--radius); cursor:pointer;">Search</button>
                    </div>
                    <div id="search-results" style="margin-top:20px;"></div>
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
                case 'dashboard': loadMetrics(); loadDashboardExtended(); break;
                case 'approvals': loadApprovals(); loadLocks(); break;
                case 'agents': loadAgents(); break;
                case 'models': loadModels(); break;
                case 'audit': loadAudit(); break;
                case 'parallel': loadParallel(); break;
                case 'swarm': loadSwarm(); break;
                case 'cloud': loadCloud(); break;
                case 'mission': loadMissionFeed(); break;
                case 'marketplace': loadMarketplace(); break;
                case 'search': checkIndexStatus(); break;
                case 'usage': loadUsage(); break;
                case 'finetune': break; // no auto-load needed
                case 'depgraph': break; // user triggers manually
                case 'events': break;   // driven by EventSource, not polling
                case 'runtemplates': loadRunTemplates(); break;
            }
        }

        async function runCodeSearch() {
            const q = document.getElementById('search-query').value.trim();
            if (!q) return;
            const limit = document.getElementById('search-limit').value;
            const resultsEl = document.getElementById('search-results');
            resultsEl.innerHTML = '<div style="color:var(--text-secondary)">Searching...</div>';
            try {
                const r = await apiFetch(`/api/search?q=${encodeURIComponent(q)}&limit=${limit}`);
                const data = await r.json();
                const results = data.results || [];
                if (!results.length) {
                    resultsEl.innerHTML = '<div style="color:var(--text-secondary)">No results found.</div>';
                    return;
                }
                resultsEl.innerHTML = results.map((item, i) => `
                    <div style="border:1px solid var(--border); border-radius:var(--radius); padding:12px 16px; margin-bottom:10px; background:var(--surface);">
                        <div style="display:flex; justify-content:space-between; align-items:center;">
                            <strong style="color:var(--accent); font-size:14px;">${item.path || ''}</strong>
                            <span style="font-size:11px; color:var(--text-secondary); background:var(--surface-solid); padding:2px 8px; border-radius:10px;">${item.language || ''}</span>
                        </div>
                        ${item.exports && item.exports.length ? `<div style="font-size:12px; color:var(--text-secondary); margin-top:6px;">exports: ${item.exports.slice(0,6).join(', ')}</div>` : ''}
                        ${item.summary ? `<div style="font-size:13px; margin-top:6px; opacity:0.8;">${item.summary.split('\\n')[0]}</div>` : ''}
                    </div>
                `).join('');
            } catch(e) {
                resultsEl.innerHTML = `<div style="color:#e74c3c">Search failed: ${e.message}</div>`;
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

        async function runComparison() {
            const prompt = document.getElementById('cmp-prompt').value.trim();
            const modelA = document.getElementById('cmp-model-a').value.trim();
            const modelB = document.getElementById('cmp-model-b').value.trim();
            if (!prompt) { alert('Enter a prompt first.'); return; }
            if (!modelA || !modelB) { alert('Enter both model names.'); return; }
            const status = document.getElementById('cmp-status');
            status.textContent = 'Running comparison on both models…';
            document.getElementById('cmp-results').style.display = 'none';

            async function runOne(model) {
                const t0 = Date.now();
                const r = await apiFetch('/api/prompt', {
                    method: 'POST',
                    body: JSON.stringify({ prompt, model, session_id: 'cmp-' + model.replace(/[^a-z0-9]/gi,'_') }),
                });
                const ms = Date.now() - t0;
                const data = await r.json();
                return { model, ms, data, ok: r.ok };
            }

            try {
                const [resA, resB] = await Promise.all([runOne(modelA), runOne(modelB)]);
                status.textContent = 'Done.';
                document.getElementById('cmp-results').style.display = 'block';

                for (const [res, suffix] of [[resA,'a'],[resB,'b']]) {
                    document.getElementById('cmp-header-' + suffix).textContent = res.model;
                    const tokens = res.data.token_usage;
                    const meta = res.ok
                        ? `${res.ms}ms · ${tokens ? tokens.prompt_tokens + ' prompt / ' + tokens.completion_tokens + ' completion tokens' : 'tokens n/a'}`
                        : 'Error';
                    document.getElementById('cmp-meta-' + suffix).textContent = meta;
                    document.getElementById('cmp-output-' + suffix).textContent = res.ok
                        ? (res.data.response || res.data.summary || JSON.stringify(res.data, null, 2))
                        : (res.data.error || 'Request failed');
                }
            } catch(e) {
                status.textContent = 'Error: ' + e.message;
            }
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
            tbody.innerHTML = '<thead><tr><th>Run ID</th><th>Status</th><th>Tasks</th><th>Cost</th><th>Actions</th></tr></thead>' +
                runs.map(run => `<tr>
                    <td><code style="font-size:11px;">${run.run_id}</code></td>
                    <td><span class="status-badge ${run.status === 'Running' ? 'status-ok' : 'status-err'}">${run.status}</span></td>
                    <td>${run.task_count}</td>
                    <td><span id="cost-${run.run_id.replace(/[^a-z0-9]/gi,'_')}" style="color:var(--muted); font-size:12px;">—</span>
                        <button onclick="fetchRunCost('${run.run_id}')" style="background:none; border:none; color:var(--accent); font-size:11px; cursor:pointer; margin-left:4px;">💰</button></td>
                    <td><button onclick="replayRun('${run.run_id}')"
                        style="background:rgba(99,102,241,0.15); border:1px solid var(--accent); color:var(--accent); padding:4px 10px; border-radius:6px; font-size:11px; cursor:pointer;">↺ Replay</button></td>
                </tr>`).join('');
        }

        async function fetchRunCost(runId) {
            try {
                const r = await apiFetch(`/api/parallel/runs/${runId}/cost`);
                const d = await r.json();
                const el = document.getElementById('cost-' + runId.replace(/[^a-z0-9]/gi, '_'));
                if (el) el.textContent = `$${(d.estimated_cost_usd || 0).toFixed(4)} · ${(d.total_tokens || 0).toLocaleString()} tok`;
            } catch(e) {}
        }

        async function replayRun(runId) {
            const btn = event.target;
            btn.disabled = true; btn.textContent = '↺ …';
            try {
                const r = await apiFetch(`/api/parallel/runs/${runId}/replay`, { method: 'POST', body: '{}' });
                const d = await r.json();
                if (r.ok) {
                    btn.textContent = `↺ ${d.id || 'queued'}`;
                    setTimeout(loadParallel, 1500);
                } else {
                    btn.textContent = '↺ Error';
                    btn.disabled = false;
                }
            } catch(e) { btn.textContent = '↺ Replay'; btn.disabled = false; }
        }

        async function launchSwarm() {
            const goal = document.getElementById('swarm-goal').value.trim();
            if (!goal) { alert('Please enter a goal.'); return; }
            const rawFiles = document.getElementById('swarm-files').value.trim();
            const files = rawFiles ? rawFiles.split('\n').map(s => s.trim()).filter(Boolean) : [];
            const statusEl = document.getElementById('swarm-launch-status');
            statusEl.textContent = 'Submitting…';
            try {
                const r = await apiFetch('/api/swarm/runs', {
                    method: 'POST',
                    body: JSON.stringify({ goal, files, use_llm_planner: true, planner_model: 'gemma4:26b' }),
                });
                const data = await r.json();
                if (r.ok) {
                    statusEl.textContent = `Launched: ${data.run_id}`;
                    document.getElementById('swarm-goal').value = '';
                    document.getElementById('swarm-files').value = '';
                    setTimeout(loadSwarm, 1000);
                } else {
                    statusEl.textContent = `Error: ${data.error || r.status}`;
                }
            } catch (e) {
                statusEl.textContent = `Error: ${e.message}`;
            }
        }

        async function loadSwarm() {
            const r = await apiFetch('/api/swarm/runs');
            const runs = await r.json();
            const list = document.getElementById('swarm-list');
            if (!Array.isArray(runs) || !runs.length) {
                list.innerHTML = '<p style="color:var(--muted); font-size: 13px;">No swarm runs yet. Use the form above to launch one.</p>';
                return;
            }
            list.innerHTML = runs.map(run => `
                <div class="card" style="margin-bottom: 12px; border-left: 4px solid var(--accent); cursor:pointer;"
                     onclick="loadDag('${run.id}')">
                    <div style="display:flex; justify-content:space-between; align-items:center;">
                        <div>
                            <div style="font-weight:600;">RUN: ${run.id}</div>
                            <div style="font-size:12px; color:var(--muted);">${run.status} · ${(run.tasks||[]).length} sub-tasks · click to visualize</div>
                        </div>
                        <div class="status-badge ${run.status === 'running' ? 'status-ok' : 'status-err'}">${run.status}</div>
                    </div>
                </div>
            `).join('');
        }

        async function loadDag(runId) {
            const r = await apiFetch('/api/swarm/runs/' + runId);
            if (!r.ok) return;
            const run = await r.json();
            renderDag(run);
        }

        function renderDag(run) {
            const tasks = run.tasks || [];
            const hint = document.getElementById('swarm-dag-hint');
            const svg = document.getElementById('swarm-dag-svg');
            if (!tasks.length) {
                hint.style.display = 'block';
                svg.style.display = 'none';
                hint.textContent = 'No tasks in this run.';
                return;
            }
            hint.style.display = 'none';
            svg.style.display = 'block';

            // Layout: topological levels
            const levels = dagLevels(tasks);
            const maxLevel = Math.max(...Object.values(levels));
            const colW = 160, rowH = 80, padX = 20, padY = 30;
            const levelCounts = {};
            tasks.forEach(t => { const l = levels[t.id] || 0; levelCounts[l] = (levelCounts[l]||0)+1; });
            const maxPerLevel = Math.max(...Object.values(levelCounts));
            const svgW = (maxLevel + 1) * colW + padX * 2;
            const svgH = maxPerLevel * rowH + padY * 2;
            svg.setAttribute('viewBox', '0 0 ' + svgW + ' ' + svgH);
            svg.setAttribute('height', svgH);

            // Compute node centres
            const centres = {};
            const levelIdx = {};
            tasks.forEach(t => {
                const l = levels[t.id] || 0;
                const idx = levelIdx[l] = (levelIdx[l]||0);
                levelIdx[l]++;
                const x = padX + l * colW + colW / 2;
                const y = padY + idx * rowH + rowH / 2;
                centres[t.id] = { x, y };
            });

            const statusColor = s => s === 'completed' ? '#4ade80' : s === 'failed' ? '#f87171' : s === 'running' ? '#facc15' : '#94a3b8';

            let out = '<defs><marker id="arrow" markerWidth="8" markerHeight="8" refX="6" refY="3" orient="auto">'
                    + '<path d="M0,0 L0,6 L8,3 z" fill="#94a3b8"/></marker></defs>';

            // Draw edges first (behind nodes)
            tasks.forEach(t => {
                (t.deps || []).forEach(dep => {
                    if (centres[dep] && centres[t.id]) {
                        const s = centres[dep], e = centres[t.id];
                        out += '<line x1="' + (s.x+50) + '" y1="' + s.y + '" x2="' + (e.x-50) + '" y2="' + e.y
                             + '" stroke="#94a3b8" stroke-width="1.5" marker-end="url(#arrow)"/>';
                    }
                });
            });

            // Draw nodes
            tasks.forEach(t => {
                const c = centres[t.id];
                const col = statusColor(t.status);
                const label = (t.id.length > 14 ? t.id.slice(0,13)+'…' : t.id);
                out += '<rect x="' + (c.x-50) + '" y="' + (c.y-22) + '" width="100" height="44" rx="8" '
                     + 'fill="rgba(0,0,0,0.5)" stroke="' + col + '" stroke-width="2"/>';
                out += '<text x="' + c.x + '" y="' + (c.y-5) + '" text-anchor="middle" font-size="11" fill="' + col + '">' + label + '</text>';
                out += '<text x="' + c.x + '" y="' + (c.y+10) + '" text-anchor="middle" font-size="9" fill="#94a3b8">' + (t.status||'pending') + '</text>';
            });

            svg.innerHTML = out;
        }

        function dagLevels(tasks) {
            // Assign topological levels (longest path from root)
            const levels = {};
            const taskMap = {};
            tasks.forEach(t => { taskMap[t.id] = t; levels[t.id] = 0; });
            // Kahn-style: process nodes in dependency order
            let changed = true;
            for (let iter = 0; iter < tasks.length + 1 && changed; iter++) {
                changed = false;
                tasks.forEach(t => {
                    (t.deps || []).forEach(dep => {
                        const proposed = (levels[dep] || 0) + 1;
                        if (proposed > (levels[t.id] || 0)) {
                            levels[t.id] = proposed;
                            changed = true;
                        }
                    });
                });
            }
            return levels;
        }

        async function submitCloudJob() {
            const name = document.getElementById('cloud-job-name').value.trim();
            const cmdRaw = document.getElementById('cloud-job-cmd').value.trim();
            if (!name) { alert('Job name is required.'); return; }
            const command = cmdRaw ? cmdRaw.split(/\s+/) : [];
            const statusEl = document.getElementById('cloud-submit-status');
            statusEl.textContent = 'Submitting…';
            try {
                const r = await apiFetch('/api/cloud/jobs', {
                    method: 'POST',
                    body: JSON.stringify({ name, command }),
                });
                const data = await r.json();
                if (r.ok) {
                    statusEl.textContent = `Submitted: ${data.id}`;
                    document.getElementById('cloud-job-name').value = '';
                    document.getElementById('cloud-job-cmd').value = '';
                    setTimeout(loadCloud, 500);
                } else {
                    statusEl.textContent = `Error: ${data.error || r.status}`;
                }
            } catch (e) {
                statusEl.textContent = `Error: ${e.message}`;
            }
        }

        async function loadCloud() {
            const r = await apiFetch('/api/cloud/jobs');
            const jobs = await r.json();
            const tbody = document.getElementById('cloud-table-body');
            document.getElementById('val-cloud-jobs').textContent = jobs.length;
            
            if (!jobs.length) {
                tbody.innerHTML = '<tr><td colspan="3" style="color:var(--muted); text-align:center;">No cloud bridge jobs active. Run "tachy deploy" to scale to AWS.</td></tr>';
                return;
            }
            
            tbody.innerHTML = '<thead><tr><th>Job ID</th><th>Status</th><th>Updated</th></tr></thead>' + 
                jobs.map(j => `<tr><td><code>${j.id}</code></td><td>${j.status}</td><td>${new Date(j.updated_at * 1000).toLocaleTimeString()}</td></tr>`).join('');
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

        // ── Dependency Graph Explorer ────────────────────────────────────────
        async function loadDepGraph() {
            const file = document.getElementById('depgraph-file').value.trim();
            if (!file) return;
            const r = await apiFetch('/api/graph?file=' + encodeURIComponent(file));
            if (!r.ok) { alert('File not found in graph.'); return; }
            const d = await r.json();

            // Summary stats
            const sumDiv = document.getElementById('depgraph-summary');
            sumDiv.style.display = 'block';
            document.getElementById('dg-imports-count').textContent = (d.direct_imports||[]).length;
            document.getElementById('dg-imported-by-count').textContent = (d.imported_by||[]).length;
            document.getElementById('dg-transitive-count').textContent = (d.transitive_dependents||[]).length;

            // SVG visualization — three-column layout: imported_by | focal | direct_imports
            const focal = d.file;
            const imports = d.direct_imports || [];
            const importedBy = d.imported_by || [];

            const card = document.getElementById('depgraph-vis-card');
            card.style.display = 'block';
            document.getElementById('depgraph-full-card').style.display = 'none';
            document.getElementById('depgraph-vis-title').textContent = 'Dependency Map: ' + focal.split('/').pop();

            const colW = 180, rowH = 44, padX = 20, padY = 20;
            const maxRows = Math.max(imports.length, importedBy.length, 1);
            const svgH = maxRows * rowH + padY * 2 + 60;
            const svgW = colW * 3 + padX * 2;
            const svg = document.getElementById('depgraph-svg');
            svg.setAttribute('viewBox', '0 0 ' + svgW + ' ' + svgH);
            svg.setAttribute('height', Math.max(svgH, 200));

            const focalX = padX + colW + colW / 2;
            const focalY = svgH / 2;

            function nodeY(i, total) {
                return padY + 30 + i * rowH + (maxRows - total) * rowH / 2;
            }

            let out = '<defs><marker id="dgarrow" markerWidth="8" markerHeight="8" refX="6" refY="3" orient="auto">'
                    + '<path d="M0,0 L0,6 L8,3 z" fill="#64748b"/></marker></defs>';

            // Focal node
            out += `<rect x="${focalX-70}" y="${focalY-14}" width="140" height="28" rx="6" fill="rgba(99,102,241,0.3)" stroke="#6366f1" stroke-width="2"/>`;
            const focalLabel = focal.length > 20 ? '…' + focal.slice(-19) : focal;
            out += `<text x="${focalX}" y="${focalY+5}" text-anchor="middle" font-size="10" fill="#e2e8f0">${focalLabel}</text>`;

            // Column labels
            out += `<text x="${padX + colW/2}" y="18" text-anchor="middle" font-size="10" fill="#64748b">imported by</text>`;
            out += `<text x="${padX + colW*2 + colW/2}" y="18" text-anchor="middle" font-size="10" fill="#64748b">imports</text>`;

            // imported_by nodes (left column)
            importedBy.forEach((f, i) => {
                const x = padX + colW / 2;
                const y = nodeY(i, importedBy.length);
                const label = f.length > 22 ? '…' + f.slice(-21) : f;
                out += `<rect x="${x-70}" y="${y-12}" width="140" height="24" rx="5" fill="rgba(0,0,0,0.4)" stroke="#475569" stroke-width="1"/>`;
                out += `<text x="${x}" y="${y+4}" text-anchor="middle" font-size="9" fill="#94a3b8">${label}</text>`;
                out += `<line x1="${x+70}" y1="${y}" x2="${focalX-70}" y2="${focalY}" stroke="#64748b" stroke-width="1" marker-end="url(#dgarrow)"/>`;
            });

            // direct_imports nodes (right column)
            imports.forEach((f, i) => {
                const x = padX + colW * 2 + colW / 2;
                const y = nodeY(i, imports.length);
                const label = f.length > 22 ? '…' + f.slice(-21) : f;
                out += `<rect x="${x-70}" y="${y-12}" width="140" height="24" rx="5" fill="rgba(0,0,0,0.4)" stroke="#475569" stroke-width="1"/>`;
                out += `<text x="${x}" y="${y+4}" text-anchor="middle" font-size="9" fill="#94a3b8">${label}</text>`;
                out += `<line x1="${focalX+70}" y1="${focalY}" x2="${x-70}" y2="${y}" stroke="#64748b" stroke-width="1" marker-end="url(#dgarrow)"/>`;
            });

            svg.innerHTML = out;
        }

        async function loadFullGraph() {
            const r = await apiFetch('/api/graph');
            if (!r.ok) return;
            const g = await r.json();
            document.getElementById('depgraph-vis-card').style.display = 'none';
            const card = document.getElementById('depgraph-full-card');
            card.style.display = 'block';
            document.getElementById('dg-total-nodes').textContent = Object.keys(g.nodes||{}).length;
            document.getElementById('dg-total-edges').textContent = g.edge_count || 0;
            const list = document.getElementById('depgraph-file-list');
            const nodes = Object.values(g.nodes || {});
            nodes.sort((a, b) => (b.imports?.length||0) - (a.imports?.length||0));
            list.innerHTML = nodes.map(n => {
                const lang = `<span style="color:var(--muted); margin-right:8px;">[${n.language}]</span>`;
                const imports = n.imports?.length ? `<span style="color:#4ade80;">↓${n.imports.length}</span>` : '';
                const importedBy = n.imported_by?.length ? `<span style="color:#60a5fa; margin-left:6px;">↑${n.imported_by.length}</span>` : '';
                return `<div style="padding:4px 0; border-bottom:1px solid var(--border); cursor:pointer;"
                    onclick="document.getElementById('depgraph-file').value='${n.path}'; loadDepGraph();">
                    ${lang}<span>${n.path}</span>${imports}${importedBy}
                </div>`;
            }).join('');
        }

        async function loadUsage() {
            try {
                const r = await apiFetch('/api/usage');
                const d = await r.json();
                document.getElementById('usage-total-tokens').textContent = (d.totals?.tokens || 0).toLocaleString();
                document.getElementById('usage-total-tools').textContent = (d.totals?.tool_invocations || 0).toLocaleString();
                document.getElementById('usage-total-runs').textContent = (d.totals?.agent_runs || 0).toLocaleString();
                const tbody = document.getElementById('usage-table');
                const users = d.users || [];
                if (!users.length) {
                    tbody.innerHTML = '<tr><td colspan="5" style="color:var(--muted); text-align:center; padding:20px;">No usage data yet. Run an agent to start metering.</td></tr>';
                } else {
                    tbody.innerHTML = '<thead><tr><th>User</th><th>Team</th><th>Tokens (in/out)</th><th>Tool Invocations</th><th>Agent Runs</th></tr></thead>'
                        + users.map(u => `<tr>
                            <td><code>${u.user_id}</code></td>
                            <td>${u.team_id || '—'}</td>
                            <td>${(u.total_input_tokens||0).toLocaleString()} / ${(u.total_output_tokens||0).toLocaleString()}</td>
                            <td>${(u.total_tool_invocations||0).toLocaleString()}</td>
                            <td>${(u.total_agent_runs||0).toLocaleString()}</td>
                        </tr>`).join('');
                }
            } catch(e) {
                document.getElementById('usage-table').innerHTML = '<tr><td style="color:var(--muted)">Error loading usage data.</td></tr>';
            }
        }

        let _finetuneJsonl = '';
        async function extractDataset() {
            const status = document.getElementById('finetune-status');
            status.textContent = 'Extracting…';
            try {
                const r = await apiFetch('/api/finetune/extract', { method: 'POST', body: JSON.stringify({}) });
                const d = await r.json();
                status.textContent = '';
                _finetuneJsonl = d.jsonl || '';
                document.getElementById('finetune-dataset-info').style.display = 'block';
                document.getElementById('finetune-stats').textContent =
                    `Extracted ${d.entries} training pairs from ${d.source_sessions} sessions.`;
            } catch(e) {
                status.textContent = 'Error: ' + e.message;
            }
        }

        function downloadJsonl() {
            if (!_finetuneJsonl) return;
            const blob = new Blob([_finetuneJsonl], { type: 'application/jsonl' });
            const url = URL.createObjectURL(blob);
            const a = document.createElement('a');
            a.href = url; a.download = 'tachy-finetune.jsonl'; a.click();
            URL.revokeObjectURL(url);
        }

        async function generateModelfile() {
            const base = document.getElementById('ft-base-model').value.trim();
            const adapter = document.getElementById('ft-adapter-path').value.trim();
            const prompt = document.getElementById('ft-system-prompt').value.trim();
            if (!base || !adapter) { alert('Base model and adapter path are required.'); return; }
            const status = document.getElementById('modelfile-status');
            status.textContent = 'Generating…';
            try {
                const r = await apiFetch('/api/finetune/modelfile', {
                    method: 'POST',
                    body: JSON.stringify({ base_model: base, adapter_path: adapter, system_prompt: prompt || undefined }),
                });
                const d = await r.json();
                status.textContent = '';
                const out = document.getElementById('modelfile-output');
                out.style.display = 'block';
                out.textContent = d.modelfile || '';
            } catch(e) {
                status.textContent = 'Error: ' + e.message;
            }
        }

        async function loadMissionFeed() {
            const r = await apiFetch('/api/mission/feed');
            const feed = await r.json();
            const list = document.getElementById('mission-feed-list');
            
            if (!feed.length) {
                list.innerHTML = '<p style="color:var(--muted); font-size: 13px;">No events in the mission bus.</p>';
                return;
            }

            list.innerHTML = feed.map(e => {
                let badge = 'status-ok';
                let icon = '📡';
                let payload = JSON.stringify(e);
                
                return `
                <div class="card" style="margin-bottom: 8px; border-left: 3px solid var(--accent); padding: 12px;">
                    <div style="display:flex; justify-content:space-between; font-size:12px; margin-bottom:4px;">
                        <span style="font-weight:600; color:var(--accent);">${icon} Event</span>
                        <span style="color:var(--muted); opacity:0.6;">${e.agent_id || 'system'}</span>
                    </div>
                    <div style="font-size:13px;">${payload}</div>
                </div>`;
            }).join('');
        }

        async function loadMarketplace() {
            const r = await apiFetch('/api/marketplace');
            const listings = await r.json();
            const pub = document.getElementById('marketplace-public');
            const team = document.getElementById('marketplace-team');
            
            pub.innerHTML = listings.filter(l => l.visibility !== 'team').map(renderListing).join('');
            team.innerHTML = listings.filter(l => l.visibility === 'team').map(renderListing).join('');
        }

        function renderListing(l) {
            return `
            <div class="card" style="background:var(--surface-solid);">
                <div style="font-weight:600; margin-bottom:8px;">${l.name}</div>
                <div style="font-size:12px; color:var(--muted); margin-bottom:12px;">${l.description}</div>
                <div style="display:flex; justify-content:space-between; align-items:center;">
                    <span style="font-size:11px; color:var(--accent);">v${l.default_version}</span>
                    <button onclick="installTemplate('${l.id}')" style="background:rgba(255,255,255,0.05); border:1px solid var(--border); color:var(--text); padding:4px 8px; border-radius:4px; font-size:11px; cursor:pointer;">Install</button>
                </div>
            </div>`;
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

        // ── C2: Index status + build ──────────────────────────────────────
        async function checkIndexStatus() {
            try {
                const r = await apiFetch('/api/index');
                const d = await r.json();
                const el = document.getElementById('index-status');
                if (!el) return;
                if (d && d.total_files !== undefined) {
                    const ago = d.built_at ? ` · built ${new Date(d.built_at * 1000).toLocaleTimeString()}` : '';
                    el.textContent = `Index: ${d.total_files} files${ago}`;
                    el.style.color = 'var(--success)';
                } else {
                    el.textContent = 'Index: not built';
                    el.style.color = 'var(--warning)';
                }
            } catch(e) {
                const el = document.getElementById('index-status');
                if (el) { el.textContent = 'Index: offline'; el.style.color = 'var(--error)'; }
            }
        }

        async function buildIndex() {
            const btn = document.getElementById('build-index-btn');
            const el  = document.getElementById('index-status');
            if (btn) { btn.disabled = true; btn.textContent = 'Building…'; }
            if (el)  el.style.color = 'var(--warning)';
            try {
                await apiFetch('/api/index', { method: 'POST', body: '{}' });
                await checkIndexStatus();
            } catch(e) {
                if (el) { el.textContent = 'Build failed'; el.style.color = 'var(--error)'; }
            } finally {
                if (btn) { btn.disabled = false; btn.textContent = 'Build Index'; }
            }
        }

        // ── E2: Extended dashboard — cost + model leaderboard ─────────────
        async function loadDashboardExtended() {
            try {
                const r = await apiFetch('/api/dashboard');
                const d = await r.json();
                const costEl = document.getElementById('val-cost-usd');
                if (costEl) costEl.textContent = '$' + (d.estimated_cost_usd || 0).toFixed(4);

                const inEl  = document.getElementById('val-input-tokens');
                const outEl = document.getElementById('val-output-tokens');
                if (inEl)  inEl.textContent  = (d.input_tokens  || 0).toLocaleString();
                if (outEl) outEl.textContent = (d.output_tokens || 0).toLocaleString();

                const tbody = document.getElementById('model-leaderboard');
                if (tbody && d.models && d.models.length) {
                    tbody.innerHTML = d.models.map((m, i) => `
                        <tr style="border-top:1px solid var(--border);">
                            <td style="padding:8px 0; color:var(--muted);">${i+1}</td>
                            <td style="padding:8px; font-weight:500;">${m.name}</td>
                            <td style="padding:8px; text-align:right; color:var(--muted);">${(m.tokens||0).toLocaleString()}</td>
                            <td style="padding:8px; text-align:right; color:var(--success);">${(m.avg_tps||0).toFixed(1)}</td>
                            <td style="padding:8px; text-align:right; color:var(--muted);">${(m.p50_ttft_ms||0).toFixed(0)}ms</td>
                            <td style="padding:8px; text-align:right;"><span style="background:rgba(99,102,241,0.15); color:var(--accent); padding:2px 6px; border-radius:4px; font-size:11px;">${m.tier||'local'}</span></td>
                        </tr>
                    `).join('');
                } else if (tbody) {
                    tbody.innerHTML = '<tr><td colspan="6" style="color:var(--muted); padding:10px 0; font-size:12px;">No model data yet — run an inference first.</td></tr>';
                }
            } catch(e) { /* daemon may be starting */ }
        }

        // ── Wave 2A: Live SSE event stream ────────────────────────────────────
        let _sseSource = null;

        function initEventStream() {
            if (_sseSource) return; // already open
            const feedEl = document.getElementById('event-feed');
            const badgeEl = document.getElementById('sse-status-badge');

            try {
                _sseSource = new EventSource('/api/events');
            } catch(e) {
                if (badgeEl) { badgeEl.textContent = '● Unavailable'; }
                return;
            }

            _sseSource.onopen = () => {
                if (badgeEl) {
                    badgeEl.textContent = '● Connected';
                    badgeEl.style.background = 'rgba(16,185,129,0.15)';
                    badgeEl.style.color = 'var(--success)';
                }
                if (feedEl && feedEl.querySelector('[style*="Waiting"]')) {
                    feedEl.innerHTML = '';
                }
            };

            _sseSource.onmessage = (ev) => {
                appendEvent(ev.data, 'message');
            };

            // Named events from publish_event() — e.g. "event: agent_run_complete"
            ['agent_run_complete','task_complete','run_replay_started','run_replay_complete',
             'template_run_started','template_run_complete','worker_heartbeat','lag'].forEach(name => {
                _sseSource.addEventListener(name, (ev) => {
                    appendEvent(ev.data, name);
                    // Reactive refresh
                    if (name === 'agent_run_complete') loadAgents();
                    if (name === 'task_complete' || name === 'run_replay_complete') loadParallel();
                });
            });

            _sseSource.onerror = () => {
                if (badgeEl) {
                    badgeEl.textContent = '● Reconnecting…';
                    badgeEl.style.background = 'rgba(245,158,11,0.15)';
                    badgeEl.style.color = 'var(--warning)';
                }
            };
        }

        function appendEvent(dataStr, kind) {
            const feedEl = document.getElementById('event-feed');
            if (!feedEl) return;
            let payload = {};
            try { payload = JSON.parse(dataStr); } catch(e) { payload = { raw: dataStr }; }
            const ts = payload.ts ? new Date(payload.ts * 1000).toLocaleTimeString() : new Date().toLocaleTimeString();
            const kindColor = {
                agent_run_complete: 'var(--success)',
                task_complete: 'var(--accent)',
                run_replay_started: 'var(--warning)',
                run_replay_complete: 'var(--success)',
                template_run_started: 'var(--accent)',
                template_run_complete: 'var(--success)',
                lag: 'var(--error)',
            }[kind] || 'var(--muted)';
            const row = document.createElement('div');
            row.style.cssText = 'padding:4px 0; border-bottom:1px solid rgba(255,255,255,0.04); display:flex; gap:12px; align-items:baseline;';
            row.innerHTML = `<span style="color:var(--muted); min-width:80px; font-size:11px;">${ts}</span>`
                + `<span style="color:${kindColor}; min-width:160px; font-size:11px; font-weight:500;">${kind}</span>`
                + `<span style="color:var(--text); font-size:11px; word-break:break-all;">${JSON.stringify(payload.payload ?? payload)}</span>`;
            feedEl.prepend(row); // newest at top
            // Keep at most 200 rows
            while (feedEl.children.length > 200) feedEl.removeChild(feedEl.lastChild);
        }

        function clearEvents() {
            const feedEl = document.getElementById('event-feed');
            if (feedEl) feedEl.innerHTML = '<div style="color:var(--muted); text-align:center; padding:40px 0;">Cleared.</div>';
        }

        // ── Wave 2D: Named DAG templates ──────────────────────────────────────
        async function loadRunTemplates() {
            try {
                const r = await apiFetch('/api/run-templates');
                const data = await r.json();
                const templates = (data.templates || []);
                const tbl = document.getElementById('runtemplates-table');
                if (!tbl) return;
                if (!templates.length) {
                    tbl.innerHTML = '<tr><td colspan="5" style="color:var(--muted); padding:20px 0; font-size:13px;">No templates saved yet. Create one above.</td></tr>';
                    return;
                }
                tbl.innerHTML = '<thead><tr><th style="text-align:left; padding:8px 0; color:var(--muted); font-size:11px;">NAME</th>'
                    + '<th style="text-align:left; padding:8px; color:var(--muted); font-size:11px;">DESCRIPTION</th>'
                    + '<th style="text-align:right; padding:8px; color:var(--muted); font-size:11px;">TASKS</th>'
                    + '<th style="text-align:right; padding:8px; color:var(--muted); font-size:11px;">CONCURRENCY</th>'
                    + '<th style="padding:8px;"></th></tr></thead>'
                    + templates.map(t => `<tr style="border-top:1px solid var(--border);">
                        <td style="padding:10px 0; font-weight:600; font-family:monospace; font-size:13px;">${t.name}</td>
                        <td style="padding:10px 8px; color:var(--muted); font-size:12px;">${t.description || '—'}</td>
                        <td style="padding:10px 8px; text-align:right;">${(t.tasks||[]).length}</td>
                        <td style="padding:10px 8px; text-align:right;">${t.max_concurrency || 4}</td>
                        <td style="padding:10px 8px; display:flex; gap:6px; justify-content:flex-end;">
                            <button onclick="executeRunTemplate('${t.name}')"
                                style="background:rgba(16,185,129,0.15); border:1px solid var(--success); color:var(--success); padding:4px 10px; border-radius:6px; font-size:11px; cursor:pointer;">▶ Run</button>
                            <button onclick="deleteRunTemplate('${t.name}')"
                                style="background:rgba(239,68,68,0.1); border:1px solid var(--error); color:var(--error); padding:4px 10px; border-radius:6px; font-size:11px; cursor:pointer;">✕</button>
                        </td>
                    </tr>`).join('');
            } catch(e) {}
        }

        async function saveRunTemplate() {
            const name = document.getElementById('tpl-name').value.trim();
            const tasksRaw = document.getElementById('tpl-tasks').value.trim();
            const statusEl = document.getElementById('tpl-save-status');
            if (!name) { statusEl.textContent = 'Name is required.'; statusEl.style.color = 'var(--error)'; return; }
            let tasks;
            try { tasks = JSON.parse(tasksRaw); } catch(e) {
                statusEl.textContent = 'Invalid JSON in tasks field.'; statusEl.style.color = 'var(--error)'; return;
            }
            statusEl.textContent = 'Saving…'; statusEl.style.color = 'var(--muted)';
            try {
                const r = await apiFetch('/api/run-templates', {
                    method: 'POST',
                    body: JSON.stringify({ name, tasks }),
                });
                const d = await r.json();
                if (r.ok) {
                    statusEl.textContent = `Saved "${d.name}"`;
                    statusEl.style.color = 'var(--success)';
                    document.getElementById('tpl-name').value = '';
                    document.getElementById('tpl-tasks').value = '';
                    loadRunTemplates();
                } else {
                    statusEl.textContent = d.error || 'Save failed.';
                    statusEl.style.color = 'var(--error)';
                }
            } catch(e) { statusEl.textContent = 'Request failed.'; statusEl.style.color = 'var(--error)'; }
        }

        async function deleteRunTemplate(name) {
            if (!confirm(`Delete template "${name}"?`)) return;
            try {
                await apiFetch(`/api/run-templates/${encodeURIComponent(name)}`, { method: 'DELETE' });
                loadRunTemplates();
            } catch(e) {}
        }

        async function executeRunTemplate(name) {
            try {
                const r = await apiFetch(`/api/run-templates/${encodeURIComponent(name)}/run`, { method: 'POST', body: '{}' });
                const d = await r.json();
                if (r.ok) {
                    alert(`Template "${name}" started — run ID: ${d.id || '(queued)'}`);
                    showPage('parallel', document.querySelector('[onclick*="parallel"]'));
                } else {
                    alert(`Error: ${d.error || r.status}`);
                }
            } catch(e) { alert(`Request failed: ${e.message}`); }
        }

        // Initialize
        (async () => {
            const health = await (await apiFetch('/health')).json();
            document.getElementById('workspace-path').textContent = health.workspace || 'Local';
            loadModels();
            loadTemplates();
            checkHealth();
            checkIndexStatus();
            loadDashboardExtended();
            initEventStream();
            setInterval(checkHealth, 5000);
            setInterval(() => { if(ttftChart) loadMetrics(); }, 3000);
            setInterval(loadDashboardExtended, 10000);
        })();
    </script>
</body>
</html>
"##;
