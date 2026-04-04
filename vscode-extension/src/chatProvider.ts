import * as vscode from 'vscode';
import { TachyClient } from './client';

export class TachyChatProvider implements vscode.WebviewViewProvider {
    private _view?: vscode.WebviewView;

    constructor(
        private readonly extensionUri: vscode.Uri,
        private readonly client: TachyClient,
        private readonly defaultModel: string,
    ) {}

    resolveWebviewView(webviewView: vscode.WebviewView) {
        this._view = webviewView;
        webviewView.webview.options = { enableScripts: true };
        webviewView.webview.html = this.getHtml();

        webviewView.webview.onDidReceiveMessage(async (message) => {
            switch (message.type) {
                case 'send':
                    await this.handleSend(message.text, message.model);
                    break;
                case 'newChat':
                    this.newChat();
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
            await this.handleSend(text, this.defaultModel);
        }
    }

    private async handleSend(text: string, model: string) {
        if (!this._view) { return; }

        this._view.webview.postMessage({ type: 'thinking' });

        try {
            const result = await this.client.runAndPoll('chat', text, model || this.defaultModel, (secs) => {
                this._view?.webview.postMessage({ type: 'progress', secs });
            });

            this._view.webview.postMessage({
                type: 'response',
                text: result.summary || 'No response.',
                iterations: result.iterations,
                toolCalls: result.tool_invocations,
                model: model || this.defaultModel,
            });
        } catch (e: any) {
            this._view.webview.postMessage({
                type: 'error',
                text: e.message || 'Failed to connect to Tachy daemon. Run: tachy serve',
            });
        }
    }

    private getHtml(): string {
        return `<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
<style>
* { margin: 0; padding: 0; box-sizing: border-box; }
body { font-family: var(--vscode-font-family); font-size: var(--vscode-font-size); color: var(--vscode-foreground); background: var(--vscode-sideBar-background); display: flex; flex-direction: column; height: 100vh; }
.messages { flex: 1; overflow-y: auto; padding: 12px; }
.msg { margin-bottom: 12px; }
.msg.user { text-align: right; }
.msg .bubble { display: inline-block; max-width: 90%; padding: 8px 12px; border-radius: 8px; text-align: left; white-space: pre-wrap; word-break: break-word; line-height: 1.5; }
.msg.user .bubble { background: var(--vscode-button-background); color: var(--vscode-button-foreground); }
.msg.assistant .bubble { background: var(--vscode-editor-background); border: 1px solid var(--vscode-panel-border); }
.msg .meta { font-size: 11px; color: var(--vscode-descriptionForeground); margin-top: 4px; }
.input-area { padding: 8px 12px; border-top: 1px solid var(--vscode-panel-border); display: flex; gap: 6px; }
.input-area input { flex: 1; background: var(--vscode-input-background); color: var(--vscode-input-foreground); border: 1px solid var(--vscode-input-border); padding: 6px 10px; border-radius: 4px; font-size: 13px; outline: none; }
.input-area button { background: var(--vscode-button-background); color: var(--vscode-button-foreground); border: none; padding: 6px 12px; border-radius: 4px; cursor: pointer; font-size: 12px; }
.input-area button:hover { background: var(--vscode-button-hoverBackground); }
.spinner { display: inline-block; width: 12px; height: 12px; border: 2px solid var(--vscode-panel-border); border-top-color: var(--vscode-button-background); border-radius: 50%; animation: spin 0.6s linear infinite; }
@keyframes spin { to { transform: rotate(360deg); } }
code { background: var(--vscode-textCodeBlock-background); padding: 1px 4px; border-radius: 3px; font-family: var(--vscode-editor-font-family); font-size: 12px; }
pre { background: var(--vscode-textCodeBlock-background); padding: 8px; border-radius: 4px; overflow-x: auto; margin: 8px 0; font-size: 12px; }
</style>
</head>
<body>
<div class="messages" id="messages">
  <div class="msg assistant"><div class="bubble">Hi! I'm Tachy, your local AI coding agent. Select code and right-click for quick actions, or ask me anything here.</div></div>
</div>
<div class="input-area">
  <input type="text" id="input" placeholder="Ask Tachy..." onkeydown="if(event.key==='Enter')send()">
  <button onclick="send()">Send</button>
</div>
<script>
const vscode = acquireVsCodeApi();

function send() {
  const input = document.getElementById('input');
  const text = input.value.trim();
  if (!text) return;
  input.value = '';
  addMsg('user', text);
  vscode.postMessage({ type: 'send', text, model: '' });
}

function addMsg(role, text, meta) {
  const div = document.createElement('div');
  div.className = 'msg ' + role;
  const rendered = role === 'assistant' ? renderMd(text) : escHtml(text);
  div.innerHTML = '<div class="bubble">' + rendered + '</div>' + (meta ? '<div class="meta">' + meta + '</div>' : '');
  document.getElementById('messages').appendChild(div);
  div.scrollIntoView({ behavior: 'smooth' });
  return div;
}

let thinkingDiv = null;

window.addEventListener('message', event => {
  const msg = event.data;
  switch (msg.type) {
    case 'thinking':
      thinkingDiv = addMsg('assistant', '<span class="spinner"></span> Thinking...');
      break;
    case 'progress':
      if (thinkingDiv) thinkingDiv.querySelector('.bubble').innerHTML = '<span class="spinner"></span> Working... (' + msg.secs + 's)';
      break;
    case 'response':
      if (thinkingDiv) thinkingDiv.remove();
      thinkingDiv = null;
      addMsg('assistant', msg.text, msg.model + ' · ' + msg.iterations + ' iterations · ' + msg.toolCalls + ' tool calls');
      break;
    case 'error':
      if (thinkingDiv) thinkingDiv.remove();
      thinkingDiv = null;
      addMsg('assistant', 'Error: ' + msg.text);
      break;
    case 'clear':
      document.getElementById('messages').innerHTML = '<div class="msg assistant"><div class="bubble">New chat. How can I help?</div></div>';
      break;
    case 'userMessage':
      addMsg('user', msg.text);
      break;
  }
});

function renderMd(text) {
  let h = escHtml(text);
  h = h.replace(/\`\`\`(\\w*)\\n([\\s\\S]*?)\`\`\`/g, '<pre><code>$2</code></pre>');
  h = h.replace(/\`([^\`]+)\`/g, '<code>$1</code>');
  h = h.replace(/\\*\\*([^*]+)\\*\\*/g, '<strong>$1</strong>');
  h = h.replace(/^[•\\-\\*] (.+)$/gm, '• $1');
  h = h.replace(/\\n/g, '<br>');
  return h;
}

function escHtml(t) { return t.replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;'); }
</script>
</body>
</html>`;
    }
}
