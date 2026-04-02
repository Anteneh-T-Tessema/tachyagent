/// Embedded web UI served from the daemon.
/// Single HTML file with everything inline — no build step, no npm, no webpack.

pub const INDEX_HTML: &str = r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Tachy — AI Agent Platform</title>
<style>
* { margin: 0; padding: 0; box-sizing: border-box; }
:root {
  --bg: #0a0a0b; --surface: #141416; --border: #2a2a2e;
  --text: #e4e4e7; --muted: #71717a; --accent: #6366f1;
  --accent-hover: #818cf8; --success: #22c55e; --warning: #f59e0b;
  --error: #ef4444; --radius: 8px;
}
body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif; background: var(--bg); color: var(--text); min-height: 100vh; }
a { color: var(--accent); text-decoration: none; }

/* Layout */
.app { display: flex; height: 100vh; }
.sidebar { width: 240px; background: var(--surface); border-right: 1px solid var(--border); padding: 16px; display: flex; flex-direction: column; }
.main { flex: 1; display: flex; flex-direction: column; overflow: hidden; }
.logo { font-size: 20px; font-weight: 700; margin-bottom: 24px; color: var(--accent); }
.nav-item { padding: 10px 12px; border-radius: var(--radius); cursor: pointer; margin-bottom: 4px; color: var(--muted); transition: all 0.15s; display: flex; align-items: center; gap: 8px; }
.nav-item:hover, .nav-item.active { background: var(--border); color: var(--text); }
.nav-section { font-size: 11px; text-transform: uppercase; color: var(--muted); margin: 16px 0 8px 12px; letter-spacing: 0.5px; }

/* Header */
.header { padding: 16px 24px; border-bottom: 1px solid var(--border); display: flex; align-items: center; justify-content: space-between; }
.header h1 { font-size: 18px; font-weight: 600; }
.status-badge { padding: 4px 10px; border-radius: 12px; font-size: 12px; font-weight: 500; }
.status-ok { background: #052e16; color: var(--success); }
.status-err { background: #450a0a; color: var(--error); }

/* Content */
.content { flex: 1; overflow-y: auto; padding: 24px; }
.card { background: var(--surface); border: 1px solid var(--border); border-radius: var(--radius); padding: 20px; margin-bottom: 16px; }
.card h3 { font-size: 14px; font-weight: 600; margin-bottom: 12px; }
.stat { display: inline-block; margin-right: 24px; }
.stat-value { font-size: 28px; font-weight: 700; color: var(--accent); }
.stat-label { font-size: 12px; color: var(--muted); }

/* Chat */
.chat-container { flex: 1; display: flex; flex-direction: column; }
.messages { flex: 1; overflow-y: auto; padding: 24px; }
.message { margin-bottom: 16px; max-width: 80%; }
.message.user { margin-left: auto; }
.message.user .bubble { background: var(--accent); color: white; border-radius: 16px 16px 4px 16px; }
.message.assistant .bubble { background: var(--surface); border: 1px solid var(--border); border-radius: 16px 16px 16px 4px; }
.bubble { padding: 12px 16px; line-height: 1.5; white-space: pre-wrap; word-break: break-word; }
.message .meta { font-size: 11px; color: var(--muted); margin-top: 4px; padding: 0 4px; }
.tool-badge { display: inline-block; background: #1e1b4b; color: var(--accent); padding: 2px 8px; border-radius: 4px; font-size: 11px; margin: 2px; }

/* Input */
.input-bar { padding: 16px 24px; border-top: 1px solid var(--border); display: flex; gap: 8px; }
.input-bar input, .input-bar select { background: var(--surface); border: 1px solid var(--border); color: var(--text); padding: 10px 14px; border-radius: var(--radius); font-size: 14px; }
.input-bar input { flex: 1; outline: none; }
.input-bar input:focus { border-color: var(--accent); }
.input-bar button { background: var(--accent); color: white; border: none; padding: 10px 20px; border-radius: var(--radius); cursor: pointer; font-weight: 500; }
.input-bar button:hover { background: var(--accent-hover); }
.input-bar button:disabled { opacity: 0.5; cursor: not-allowed; }

/* Tables */
table { width: 100%; border-collapse: collapse; font-size: 13px; }
th { text-align: left; padding: 8px 12px; color: var(--muted); font-weight: 500; border-bottom: 1px solid var(--border); }
td { padding: 8px 12px; border-bottom: 1px solid var(--border); }
tr:hover td { background: rgba(99,102,241,0.05); }

/* Agent run */
.agent-form { display: flex; gap: 8px; margin-top: 12px; flex-wrap: wrap; }
.agent-form select, .agent-form input { background: var(--surface); border: 1px solid var(--border); color: var(--text); padding: 8px 12px; border-radius: var(--radius); font-size: 13px; }
.agent-form input { flex: 1; min-width: 200px; }
.agent-form button { background: var(--accent); color: white; border: none; padding: 8px 16px; border-radius: var(--radius); cursor: pointer; }

.spinner { display: inline-block; width: 16px; height: 16px; border: 2px solid var(--border); border-top-color: var(--accent); border-radius: 50%; animation: spin 0.6s linear infinite; }
@keyframes spin { to { transform: rotate(360deg); } }

/* Responsive */
@media (max-width: 768px) {
  .sidebar { display: none; }
  .message { max-width: 95%; }
}
</style>
</head>
<body>
<div class="app">
  <div class="sidebar">
    <div class="logo">⚡ Tachy</div>
    <div class="nav-item active" onclick="showPage('chat')">💬 Chat</div>
    <div class="nav-item" onclick="showPage('agents')">🤖 Agents</div>
    <div class="nav-item" onclick="showPage('models')">🧠 Models</div>
    <div class="nav-item" onclick="showPage('audit')">📋 Audit Log</div>
    <div class="nav-section">System</div>
    <div class="nav-item" onclick="showPage('dashboard')">📊 Dashboard</div>
  </div>
  <div class="main">
    <div class="header">
      <h1 id="page-title">Chat</h1>
      <span id="status-badge" class="status-badge status-ok">● Connected</span>
    </div>

    <!-- Chat Page -->
    <div id="page-chat" class="chat-container">
      <div class="messages" id="messages">
        <div class="message assistant"><div class="bubble">Hi! I'm Tachy, your AI assistant. I can read files, run commands, search your codebase, and more — all running locally on your machine. What would you like to do?</div></div>
      </div>
      <div class="input-bar">
        <select id="model-select"></select>
        <input type="text" id="chat-input" placeholder="Ask anything... (e.g. 'review my auth code for security issues')" onkeydown="if(event.key==='Enter')sendMessage()">
        <button onclick="sendMessage()" id="send-btn">Send</button>
      </div>
    </div>

    <!-- Agents Page -->
    <div id="page-agents" class="content" style="display:none">
      <div class="card">
        <h3>Run an Agent</h3>
        <p style="color:var(--muted);font-size:13px;margin-bottom:12px">Select a template and describe what you want the agent to do.</p>
        <div class="agent-form">
          <select id="agent-template"></select>
          <input type="text" id="agent-prompt" placeholder="e.g. 'Review the authentication module for security issues'">
          <button onclick="runAgent()" id="run-agent-btn">Run Agent</button>
        </div>
      </div>
      <div class="card">
        <h3>Agent History</h3>
        <table><thead><tr><th>ID</th><th>Template</th><th>Status</th><th>Iterations</th><th>Tools</th><th>Summary</th></tr></thead>
        <tbody id="agents-table"></tbody></table>
      </div>
    </div>

    <!-- Models Page -->
    <div id="page-models" class="content" style="display:none">
      <div class="card">
        <h3>Available Models</h3>
        <table><thead><tr><th>Model</th><th>Backend</th><th>Context</th><th>Tools</th></tr></thead>
        <tbody id="models-table"></tbody></table>
      </div>
    </div>

    <!-- Audit Page -->
    <div id="page-audit" class="content" style="display:none">
      <div class="card">
        <h3>Audit Trail</h3>
        <p style="color:var(--muted);font-size:13px;margin-bottom:12px">Every agent action is logged. Audit data is stored in <code>.tachy/audit.jsonl</code></p>
        <table><thead><tr><th>Time</th><th>Event</th><th>Agent</th><th>Tool</th><th>Detail</th></tr></thead>
        <tbody id="audit-table"><tr><td colspan="5" style="color:var(--muted)">Audit log is append-only on disk. API endpoint coming soon.</td></tr></tbody></table>
      </div>
    </div>

    <!-- Dashboard Page -->
    <div id="page-dashboard" class="content" style="display:none">
      <div style="display:flex;gap:16px;flex-wrap:wrap;margin-bottom:16px">
        <div class="card" style="flex:1;min-width:150px"><div class="stat"><div class="stat-value" id="stat-models">-</div><div class="stat-label">Models</div></div></div>
        <div class="card" style="flex:1;min-width:150px"><div class="stat"><div class="stat-value" id="stat-agents">-</div><div class="stat-label">Agents Run</div></div></div>
        <div class="card" style="flex:1;min-width:150px"><div class="stat"><div class="stat-value" id="stat-tasks">-</div><div class="stat-label">Scheduled Tasks</div></div></div>
      </div>
      <div class="card">
        <h3>Scheduled Tasks</h3>
        <table><thead><tr><th>ID</th><th>Name</th><th>Schedule</th><th>Status</th><th>Runs</th></tr></thead>
        <tbody id="tasks-table"></tbody></table>
      </div>
    </div>
  </div>
</div>

<script>
const API = '';
let currentPage = 'chat';
let apiKey = new URLSearchParams(window.location.search).get('key') || localStorage.getItem('tachy_api_key') || '';
if (apiKey) localStorage.setItem('tachy_api_key', apiKey);

function authHeaders() {
  const h = {'Content-Type': 'application/json'};
  if (apiKey) h['Authorization'] = 'Bearer ' + apiKey;
  return h;
}

function authFetch(url, opts) {
  opts = opts || {};
  opts.headers = Object.assign(authHeaders(), opts.headers || {});
  return fetch(url, opts);
}

// Navigation
function showPage(page) {
  document.querySelectorAll('.nav-item').forEach(el => el.classList.remove('active'));
  event.target.classList.add('active');
  ['chat','agents','models','audit','dashboard'].forEach(p => {
    const el = document.getElementById('page-' + p);
    if (el) el.style.display = p === page ? (p === 'chat' ? 'flex' : 'block') : 'none';
  });
  document.getElementById('page-title').textContent = {chat:'Chat',agents:'Agents',models:'Models',audit:'Audit Log',dashboard:'Dashboard'}[page];
  currentPage = page;
  if (page === 'models') loadModels();
  if (page === 'agents') loadAgents();
  if (page === 'dashboard') loadDashboard();
}

// Health check
async function checkHealth() {
  try {
    const r = await authFetch(API + '/health');
    const d = await r.json();
    document.getElementById('status-badge').className = 'status-badge status-ok';
    document.getElementById('status-badge').textContent = '● Connected';
    return d;
  } catch(e) {
    document.getElementById('status-badge').className = 'status-badge status-err';
    document.getElementById('status-badge').textContent = '● Disconnected';
    return null;
  }
}

// Load models into select and table
async function loadModels() {
  try {
    const r = await authFetch(API + '/api/models');
    const models = await r.json();
    const select = document.getElementById('model-select');
    if (select.options.length <= 1) {
      select.innerHTML = '';
      models.forEach(m => {
        const opt = document.createElement('option');
        opt.value = m.name;
        opt.textContent = m.name;
        select.appendChild(opt);
      });
      // Default to best available Ollama model
      const preferred = ['qwen3-coder:30b','qwen3:8b','llama3.1:8b','llama3.1:latest','mistral:7b'];
      const ollama = models.filter(m => m.backend === 'Ollama');
      for (const pref of preferred) {
        if (ollama.find(m => m.name === pref)) { select.value = pref; break; }
      }
    }
    const tbody = document.getElementById('models-table');
    if (tbody) {
      tbody.innerHTML = models.map(m =>
        `<tr><td>${m.name}</td><td>${m.backend}</td><td>${m.context_window.toLocaleString()}</td><td>${m.supports_tool_use ? '✓' : '—'}</td></tr>`
      ).join('');
    }
  } catch(e) { console.error('Failed to load models', e); }
}

// Load templates
async function loadTemplates() {
  try {
    const r = await authFetch(API + '/api/templates');
    const templates = await r.json();
    const select = document.getElementById('agent-template');
    select.innerHTML = '';
    templates.forEach(t => {
      const opt = document.createElement('option');
      opt.value = t.name;
      opt.textContent = t.name + ' — ' + t.description;
      select.appendChild(opt);
    });
  } catch(e) { console.error('Failed to load templates', e); }
}

// Load agents
async function loadAgents() {
  try {
    const r = await authFetch(API + '/api/agents');
    const agents = await r.json();
    const tbody = document.getElementById('agents-table');
    if (agents.length === 0) {
      tbody.innerHTML = '<tr><td colspan="6" style="color:var(--muted)">No agents run yet</td></tr>';
      return;
    }
    tbody.innerHTML = agents.map(a =>
      `<tr><td>${a.id}</td><td>${a.template}</td><td>${a.status}</td><td>${a.iterations}</td><td>${a.tool_invocations}</td><td style="max-width:300px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap">${(a.summary||'').substring(0,100)}</td></tr>`
    ).join('');
  } catch(e) { console.error('Failed to load agents', e); }
}

// Dashboard
async function loadDashboard() {
  const health = await checkHealth();
  if (health) {
    document.getElementById('stat-models').textContent = health.models;
    document.getElementById('stat-agents').textContent = health.agents;
    document.getElementById('stat-tasks').textContent = health.tasks;
  }
  try {
    const r = await authFetch(API + '/api/tasks');
    const tasks = await r.json();
    const tbody = document.getElementById('tasks-table');
    if (tasks.length === 0) {
      tbody.innerHTML = '<tr><td colspan="5" style="color:var(--muted)">No scheduled tasks</td></tr>';
    } else {
      tbody.innerHTML = tasks.map(t =>
        `<tr><td>${t.id}</td><td>${t.name}</td><td>${t.schedule}</td><td>${t.status}</td><td>${t.run_count}</td></tr>`
      ).join('');
    }
  } catch(e) {}
}

// Chat
function addMessage(role, text, meta) {
  const div = document.createElement('div');
  div.className = 'message ' + role;
  let rendered = role === 'assistant' ? renderMarkdown(text) : escapeHtml(text);
  let html = '<div class="bubble">' + rendered + '</div>';
  if (meta) html += '<div class="meta">' + meta + '</div>';
  div.innerHTML = html;
  document.getElementById('messages').appendChild(div);
  div.scrollIntoView({ behavior: 'smooth' });
}

function renderMarkdown(text) {
  // Simple markdown rendering for assistant messages
  let html = escapeHtml(text);
  // Code blocks
  html = html.replace(/```(\w*)\n([\s\S]*?)```/g, '<pre style="background:#1e1e2e;padding:12px;border-radius:6px;overflow-x:auto;margin:8px 0;font-size:13px"><code>$2</code></pre>');
  // Inline code
  html = html.replace(/`([^`]+)`/g, '<code style="background:#1e1e2e;padding:2px 6px;border-radius:4px;font-size:13px">$1</code>');
  // Bold
  html = html.replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>');
  // Bullet lists
  html = html.replace(/^[•\-\*] (.+)$/gm, '<div style="padding-left:16px">• $1</div>');
  // Numbered lists
  html = html.replace(/^(\d+)\. (.+)$/gm, '<div style="padding-left:16px">$1. $2</div>');
  // Line breaks
  html = html.replace(/\n/g, '<br>');
  return html;
}

async function sendMessage() {
  const input = document.getElementById('chat-input');
  const text = input.value.trim();
  if (!text) return;
  input.value = '';

  addMessage('user', text);

  const btn = document.getElementById('send-btn');
  btn.disabled = true;
  btn.innerHTML = '<span class="spinner"></span>';

  const model = document.getElementById('model-select').value;

  // Create assistant message placeholder
  const div = document.createElement('div');
  div.className = 'message assistant';
  div.innerHTML = '<div class="bubble"><span class="spinner"></span> Thinking...</div>';
  document.getElementById('messages').appendChild(div);
  div.scrollIntoView({ behavior: 'smooth' });

  try {
    const r = await authFetch(API + '/api/chat/stream', {
      method: 'POST',
      body: JSON.stringify({ template: 'chat', prompt: text, model: model })
    });

    const reader = r.body.getReader();
    const decoder = new TextDecoder();
    let fullText = '';
    let meta = {};

    while (true) {
      const { done, value } = await reader.read();
      if (done) break;

      const chunk = decoder.decode(value, { stream: true });
      const lines = chunk.split('\n');

      for (let i = 0; i < lines.length; i++) {
        const line = lines[i];
        if (line.startsWith('event: ')) {
          const eventType = line.substring(7);
          const dataLine = lines[i + 1];
          if (dataLine && dataLine.startsWith('data: ')) {
            const data = dataLine.substring(6);
            try {
              const parsed = JSON.parse(data);
              if (eventType === 'token') {
                fullText += parsed.text || '';
                div.innerHTML = '<div class="bubble">' + renderMarkdown(fullText) + '</div>';
                div.scrollIntoView({ behavior: 'smooth' });
              } else if (eventType === 'done') {
                meta = parsed;
              } else if (eventType === 'error') {
                fullText = 'Error: ' + (parsed.error || 'unknown');
              }
            } catch(e) {}
          }
        }
      }
    }

    if (!fullText) fullText = 'No response received';
    const metaStr = `${model} · ${meta.iterations || 0} iteration${(meta.iterations||0)!==1?'s':''} · ${meta.tool_invocations || 0} tool call${(meta.tool_invocations||0)!==1?'s':''}`;
    div.innerHTML = '<div class="bubble">' + renderMarkdown(fullText) + '</div><div class="meta">' + metaStr + '</div>';

  } catch(e) {
    div.innerHTML = '<div class="bubble">Connection error: ' + escapeHtml(e.message) + '</div>';
  }

  btn.disabled = false;
  btn.textContent = 'Send';
}

// Run agent from agents page
async function runAgent() {
  const template = document.getElementById('agent-template').value;
  const prompt = document.getElementById('agent-prompt').value.trim();
  if (!prompt) return;

  const btn = document.getElementById('run-agent-btn');
  btn.disabled = true;
  btn.innerHTML = '<span class="spinner"></span> Running...';

  try {
    const r = await authFetch(API + '/api/agents/run', {
      method: 'POST',
      headers: {'Content-Type': 'application/json'},
      body: JSON.stringify({ template, prompt })
    });
    await r.json();
    loadAgents();
  } catch(e) { alert('Error: ' + e.message); }

  btn.disabled = false;
  btn.textContent = 'Run Agent';
  document.getElementById('agent-prompt').value = '';
}

function escapeHtml(text) {
  return text.replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;');
}

// Init
checkHealth();
loadModels();
loadTemplates();
setInterval(checkHealth, 30000);
</script>
</body>
</html>
"##;
