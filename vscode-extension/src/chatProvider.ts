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

        let fullText = "";
        try {
            await this.client.streamChat(
                text,
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
  <div class="msg assistant"><div class="bubble">Hi! I'm Tachy. Select code and use the right-click menu, or ask me anything here.</div></div>
</div>

<div class="input-area">
  <textarea id="input" placeholder="Ask Tachy... (Enter to send, Shift+Enter for newline)" onkeydown="handleKey(event)"></textarea>
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

function handleKey(e) {
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
