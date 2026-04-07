import * as vscode from 'vscode';
import { TachyClient } from './client';

export class TachyChatProvider implements vscode.WebviewViewProvider {
    private _view?: vscode.WebviewView;
    private _currentModel: string;

    constructor(
        private readonly client: TachyClient,
        defaultModel: string,
    ) {
        this._currentModel = defaultModel;
    }

    resolveWebviewView(webviewView: vscode.WebviewView) {
        this._view = webviewView;
        webviewView.webview.options = { enableScripts: true };
        webviewView.webview.html = this.getHtml();

        webviewView.webview.onDidReceiveMessage(async (message) => {
            switch (message.type) {
                case 'send':
                    await this.handleSend(message.text, message.model || this._currentModel);
                    break;
                case 'newChat':
                    this.newChat();
                    break;
                case 'pickModel':
                    await vscode.commands.executeCommand('tachy.selectModel');
                    // After pick, push the new model back to the webview
                    const newModel = this.client.getModel();
                    this._currentModel = newModel;
                    this._view?.webview.postMessage({ type: 'modelChanged', model: newModel });
                    break;
            }
        });
    }

    newChat() {
        if (this._view) {
            this._view.webview.postMessage({ type: 'clear' });
        }
    }

    async sendMessage(text: string) {
        if (this._view) {
            this._view.webview.postMessage({ type: 'userMessage', text });
            await this.handleSend(text, this._currentModel);
        }
    }

    private async handleSend(text: string, model: string) {
        if (!this._view) { return; }
        this._currentModel = model;

        this._view.webview.postMessage({ type: 'thinking' });

        // ── Slash command expansion ───────────────────────────────────────
        const trimmed = text.trim();
        let effectiveText = text;

        // Handle /help inline — no daemon call needed
        if (trimmed === '/help') {
            this._view.webview.postMessage({
                type: 'response',
                text: '**Slash commands**\n\n`/fix [desc]` — fix a bug in the selected code\n`/explain [target]` — explain what code does in plain English\n`/review` — detailed code review with improvement suggestions\n`/test` — run the test suite and analyze failures\n`/commit [msg]` — commit current git changes with an optional message\n`/help` — show this message',
                iterations: 0,
                toolCalls: 0,
                model,
            });
            return;
        }

        if (trimmed.startsWith('/fix') || trimmed.startsWith('/explain') || trimmed === '/review'
            || trimmed === '/test' || trimmed.startsWith('/commit')) {
            const editor = vscode.window.activeTextEditor;
            let fileContext = '';
            if (editor) {
                const selection = editor.selection;
                const selectedText = editor.document.getText(selection);
                const relPath = vscode.workspace.asRelativePath(editor.document.uri);
                if (selectedText.trim()) {
                    fileContext = `\n\nFile: ${relPath}\n\`\`\`${editor.document.languageId}\n${selectedText}\n\`\`\``;
                } else {
                    // Use surrounding context (±30 lines around cursor)
                    const line = selection.active.line;
                    const start = Math.max(0, line - 30);
                    const end = Math.min(editor.document.lineCount - 1, line + 30);
                    const ctx = editor.document.getText(
                        new vscode.Range(new vscode.Position(start, 0), new vscode.Position(end, 0))
                    );
                    fileContext = `\n\nFile: ${relPath} (lines ${start + 1}–${end + 1})\n\`\`\`${editor.document.languageId}\n${ctx}\n\`\`\``;
                }
            }

            if (trimmed.startsWith('/fix')) {
                const desc = trimmed.slice(4).trim();
                effectiveText = desc
                    ? `Fix this issue: ${desc}${fileContext}`
                    : `Find and fix the most obvious bug or issue in the following code. Explain what you changed and why.${fileContext}`;
            } else if (trimmed.startsWith('/explain')) {
                const target = trimmed.slice(8).trim();
                effectiveText = target
                    ? `Explain \`${target}\` in plain English — its purpose, key logic, and how it fits into the codebase.${fileContext}`
                    : `Explain what this code does in plain English — its purpose, key functions, and how it fits together.${fileContext}`;
            } else if (trimmed === '/review') {
                effectiveText = `Review this code. Provide: 1) Summary of what it does, 2) Potential bugs or issues, 3) Style feedback, 4) Concrete improvement suggestions.${fileContext}`;
            } else if (trimmed === '/test') {
                effectiveText = `Run the project's test suite with full output. If any tests fail, analyze each failure and explain exactly what needs to be fixed.${fileContext}`;
            } else if (trimmed.startsWith('/commit')) {
                const commitMsg = trimmed.slice(7).trim();
                effectiveText = commitMsg
                    ? `Show the current git diff, then stage and commit all changes with this message: "${commitMsg}".`
                    : `Show the current git diff summary, write a clear conventional commit message, and stage and commit all changes.`;
            }
        }

        let fullText = "";
        try {
            await this.client.streamChat(
                effectiveText,
                model,
                (token) => {
                    fullText += token;
                    this._view?.webview.postMessage({ 
                        type: 'token', 
                        text: fullText 
                    });
                }
            );

            this._view.webview.postMessage({
                type: 'response',
                text: fullText,
                iterations: 1, // streaming chat is usually 1-shot or managed by daemon
                toolCalls: 0,
                model,
            });
        } catch (e: any) {
            this._view.webview.postMessage({
                type: 'error',
                text: e.message || 'Failed to connect to Tachy daemon. Run: tachy serve',
            });
        }
    }

    private getHtml(): string {
        const initialModel = this._currentModel;

        return `<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
<style>
* { margin: 0; padding: 0; box-sizing: border-box; }
body { font-family: var(--vscode-font-family); font-size: var(--vscode-font-size); color: var(--vscode-foreground); background: var(--vscode-sideBar-background); display: flex; flex-direction: column; height: 100vh; }

/* Header */
.header { display: flex; align-items: center; justify-content: space-between; padding: 6px 10px; border-bottom: 1px solid var(--vscode-panel-border); background: var(--vscode-editor-background); flex-shrink: 0; }
.header-title { font-weight: 600; font-size: 12px; letter-spacing: 0.03em; }
.header-actions { display: flex; gap: 4px; }
.icon-btn { background: none; border: none; cursor: pointer; color: var(--vscode-icon-foreground); padding: 3px 5px; border-radius: 3px; font-size: 13px; line-height: 1; }
.icon-btn:hover { background: var(--vscode-toolbar-hoverBackground); }

/* Model badge */
.model-bar { display: flex; align-items: center; gap: 6px; padding: 4px 10px; border-bottom: 1px solid var(--vscode-panel-border); background: var(--vscode-sideBar-background); flex-shrink: 0; }
.model-badge { font-size: 11px; color: var(--vscode-descriptionForeground); cursor: pointer; display: flex; align-items: center; gap: 4px; padding: 2px 6px; border-radius: 3px; border: 1px solid var(--vscode-panel-border); }
.model-badge:hover { background: var(--vscode-toolbar-hoverBackground); }
.model-dot { width: 7px; height: 7px; border-radius: 50%; flex-shrink: 0; background: #ce9178; }

/* Messages */
.messages { flex: 1; overflow-y: auto; padding: 12px; }
.msg { margin-bottom: 12px; }
.msg.user { text-align: right; }
.msg .bubble { display: inline-block; max-width: 90%; padding: 8px 12px; border-radius: 8px; text-align: left; white-space: pre-wrap; word-break: break-word; line-height: 1.5; }
.msg.user .bubble { background: var(--vscode-button-background); color: var(--vscode-button-foreground); }
.msg.assistant .bubble { background: var(--vscode-editor-background); border: 1px solid var(--vscode-panel-border); }
.msg .meta { font-size: 11px; color: var(--vscode-descriptionForeground); margin-top: 4px; }

/* Input */
.input-area { padding: 8px 10px; border-top: 1px solid var(--vscode-panel-border); display: flex; gap: 6px; flex-shrink: 0; }
.input-area textarea { flex: 1; background: var(--vscode-input-background); color: var(--vscode-input-foreground); border: 1px solid var(--vscode-input-border); padding: 6px 10px; border-radius: 4px; font-size: 13px; outline: none; resize: none; height: 60px; font-family: inherit; line-height: 1.4; }
.input-area textarea:focus { border-color: var(--vscode-focusBorder); }
.send-btn { background: var(--vscode-button-background); color: var(--vscode-button-foreground); border: none; padding: 6px 12px; border-radius: 4px; cursor: pointer; font-size: 12px; align-self: flex-end; }
.send-btn:hover { background: var(--vscode-button-hoverBackground); }

/* Spinner */
.spinner { display: inline-block; width: 12px; height: 12px; border: 2px solid var(--vscode-panel-border); border-top-color: var(--vscode-button-background); border-radius: 50%; animation: spin 0.6s linear infinite; }
@keyframes spin { to { transform: rotate(360deg); } }

/* Code */
code { background: var(--vscode-textCodeBlock-background); padding: 1px 4px; border-radius: 3px; font-family: var(--vscode-editor-font-family); font-size: 12px; }
pre { background: var(--vscode-textCodeBlock-background); padding: 8px; border-radius: 4px; overflow-x: auto; margin: 8px 0; font-size: 12px; }

/* Slash command menu */
.slash-menu { position: absolute; bottom: 78px; left: 0; right: 0; background: var(--vscode-editorWidget-background, var(--vscode-editor-background)); border: 1px solid var(--vscode-panel-border); border-radius: 6px 6px 0 0; border-bottom: none; overflow: hidden; z-index: 10; display: none; }
.slash-item { padding: 6px 10px; cursor: pointer; font-size: 12px; display: flex; gap: 10px; align-items: baseline; }
.slash-item:hover, .slash-item.selected { background: var(--vscode-list-activeSelectionBackground); color: var(--vscode-list-activeSelectionForeground); }
.slash-cmd { font-weight: 600; font-family: var(--vscode-editor-font-family); min-width: 80px; }
.slash-desc { color: var(--vscode-descriptionForeground); font-size: 11px; }
</style>
</head>
<body>

<div class="header">
  <span class="header-title">⚡ Tachy</span>
  <div class="header-actions">
    <button class="icon-btn" onclick="newChat()" title="New chat">＋</button>
  </div>
</div>

<div class="model-bar">
  <div class="model-badge" id="modelBadge" onclick="pickModel()" title="Click to change model">
    <span class="model-dot" id="modelDot"></span>
    <span id="modelName">${initialModel}</span>
    <span style="opacity:0.5">▾</span>
  </div>
</div>

<div class="messages" id="messages">
  <div class="msg assistant"><div class="bubble">Hi! I'm Tachy. Ask me anything, or type <code>/</code> for commands: <code>/fix</code> · <code>/explain</code> · <code>/review</code> · <code>/test</code> · <code>/commit</code></div></div>
</div>

<div class="input-area" style="position:relative;">
  <div id="slash-menu" class="slash-menu"></div>
  <textarea id="input" placeholder="Ask anything, or type / for commands…" onkeydown="handleKey(event)" oninput="handleInput(event)"></textarea>
  <button class="send-btn" onclick="send()">Send</button>
</div>

<script>
const vscode = acquireVsCodeApi();
let currentModel = ${JSON.stringify(initialModel)};

function updateModelBadge(model) {
  currentModel = model;
  document.getElementById('modelName').textContent = model;
}

// Initialize badge
updateModelBadge(currentModel);

const SLASH_CMDS = [
  { cmd: '/fix',     desc: 'Fix a bug in the selected code' },
  { cmd: '/explain', desc: 'Explain what the selected code does' },
  { cmd: '/review',  desc: 'Code review with improvement suggestions' },
  { cmd: '/test',    desc: 'Run tests and analyze failures' },
  { cmd: '/commit',  desc: 'Commit current git changes' },
  { cmd: '/help',    desc: 'Show all slash commands' },
];
let slashIdx = -1;

function handleInput(e) {
  const val = e.target.value;
  const menu = document.getElementById('slash-menu');
  const word = val.split('\n')[0];
  if (!word.startsWith('/') || word.includes(' ')) {
    menu.style.display = 'none'; return;
  }
  const filtered = SLASH_CMDS.filter(function(c) { return c.cmd.startsWith(word); });
  if (!filtered.length) { menu.style.display = 'none'; return; }
  slashIdx = -1;
  menu.innerHTML = filtered.map(function(c) {
    return '<div class="slash-item" data-cmd="' + c.cmd + '" onmousedown="pickSlash(\'' + c.cmd + '\')">' +
      '<span class="slash-cmd">' + c.cmd + '</span>' +
      '<span class="slash-desc">' + c.desc + '</span></div>';
  }).join('');
  menu.style.display = 'block';
}

function pickSlash(cmd) {
  const input = document.getElementById('input');
  input.value = cmd + ' ';
  document.getElementById('slash-menu').style.display = 'none';
  input.focus();
}

function handleKey(e) {
  const menu = document.getElementById('slash-menu');
  if (menu.style.display === 'block') {
    const items = menu.querySelectorAll('.slash-item');
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      slashIdx = Math.min(slashIdx + 1, items.length - 1);
      items.forEach(function(el, i) { el.classList.toggle('selected', i === slashIdx); });
      return;
    }
    if (e.key === 'ArrowUp') {
      e.preventDefault();
      slashIdx = Math.max(slashIdx - 1, -1);
      items.forEach(function(el, i) { el.classList.toggle('selected', i === slashIdx); });
      return;
    }
    if ((e.key === 'Tab' || e.key === 'Enter') && slashIdx >= 0) {
      e.preventDefault();
      pickSlash(menu.querySelectorAll('.slash-item')[slashIdx].dataset.cmd);
      return;
    }
    if (e.key === 'Escape') {
      menu.style.display = 'none';
      return;
    }
  }
  if (e.key === 'Enter' && !e.shiftKey) {
    e.preventDefault();
    send();
  }
}

function send() {
  const input = document.getElementById('input');
  const text = input.value.trim();
  if (!text) { return; }
  input.value = '';
  document.getElementById('slash-menu').style.display = 'none';
  addMsg('user', text);
  vscode.postMessage({ type: 'send', text, model: currentModel });
}

function pickModel() {
  vscode.postMessage({ type: 'pickModel' });
}

function newChat() {
  vscode.postMessage({ type: 'newChat' });
}

function addMsg(role, text, meta) {
  const div = document.createElement('div');
  div.className = 'msg ' + role;
  const rendered = role === 'assistant' ? renderMd(text) : escHtml(text);
  div.innerHTML = '<div class="bubble">' + rendered + '</div>'
    + (meta ? '<div class="meta">' + escHtml(meta) + '</div>' : '');
  document.getElementById('messages').appendChild(div);
  div.scrollIntoView({ behavior: 'smooth' });
  return div;
}

let thinkingDiv = null;
let streamingDiv = null;

window.addEventListener('message', (event) => {
  const msg = event.data;
  switch (msg.type) {
    case 'thinking':
      thinkingDiv = addMsg('assistant', '<span class="spinner"></span> Thinking...');
      break;
    case 'token':
      if (thinkingDiv) { thinkingDiv.remove(); thinkingDiv = null; }
      if (!streamingDiv) {
        streamingDiv = addMsg('assistant', msg.text);
      } else {
        streamingDiv.querySelector('.bubble').innerHTML = renderMd(msg.text);
        streamingDiv.scrollIntoView({ behavior: 'auto' });
      }
      break;
    case 'response':
      if (thinkingDiv) { thinkingDiv.remove(); thinkingDiv = null; }
      if (streamingDiv) {
        // Finalize
        streamingDiv.querySelector('.bubble').innerHTML = renderMd(msg.text);
        const meta = streamingDiv.querySelector('.meta') || document.createElement('div');
        meta.className = 'meta';
        meta.textContent = msg.model + ' · DONE';
        if (!streamingDiv.querySelector('.meta')) streamingDiv.appendChild(meta);
        streamingDiv = null;
      } else {
        addMsg('assistant', msg.text,
          msg.model + ' · ' + msg.iterations + ' iter · ' + msg.toolCalls + ' tools');
      }
      break;
    case 'error':
      if (thinkingDiv) { thinkingDiv.remove(); thinkingDiv = null; }
      addMsg('assistant', '⚠ ' + msg.text);
      break;
    case 'clear':
      document.getElementById('messages').innerHTML =
        '<div class="msg assistant"><div class="bubble">New chat. How can I help?</div></div>';
      break;
    case 'userMessage':
      addMsg('user', msg.text);
      break;
    case 'modelChanged':
      updateModelBadge(msg.model);
      break;
  }
});

function renderMd(text) {
  let h = escHtml(text);
  h = h.replace(/\`\`\`(\w*)\n([\s\S]*?)\`\`\`/g, '<pre><code>$2</code></pre>');
  h = h.replace(/\`([^\`]+)\`/g, '<code>$1</code>');
  h = h.replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>');
  h = h.replace(/^[•\-\*] (.+)$/gm, '• $1');
  h = h.replace(/\n/g, '<br>');
  return h;
}

function escHtml(t) {
  return String(t)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;');
}
</script>
</body>
</html>`;
    }
}
