import * as vscode from "vscode";
import * as http from "http";
import * as https from "https";

export interface HealthResponse {
  status: string;
  models: number;
}

export interface CompletionRequest {
  prefix: string;
  suffix: string;
  language: string;
  filePath: string;
  maxTokens: number;
}

export interface AgentRunResult {
  agent_id: string;
  success: boolean;
  iterations: number;
  tool_invocations: number;
  summary: string;
}

export interface ModelInfo {
  name: string;
  backend: string;
  context_window: number;
  tier: string;
  notes?: string;
}

export class TachyClient {
  private endpoint: string;
  private apiKey: string;
  private model: string;
  private lastLatencyMs = 0;

  constructor() {
    this.endpoint = "";
    this.apiKey = "";
    this.model = "";
    this.reloadConfig();
  }

  reloadConfig() {
    const config = vscode.workspace.getConfiguration("tachy");
    this.endpoint = config.get<string>("endpoint", "http://localhost:7777");
    this.apiKey = config.get<string>("apiKey", "");
    this.model = config.get<string>("model", "gemma4:26b");
  }

  getModel(): string {
    return this.model;
  }

  /** Returns the round-trip latency of the last complete() call in ms. */
  getLastLatencyMs(): number {
    return this.lastLatencyMs;
  }

  async health(): Promise<HealthResponse> {
    const data = await this.get("/health");
    return JSON.parse(data);
  }

  async listModels(): Promise<ModelInfo[]> {
    try {
      const data = await this.get("/api/models");
      const parsed = JSON.parse(data);
      return parsed.models ?? [];
    } catch {
      return [];
    }
  }

  /**
   * Run an agent task and poll until it completes.
   * Calls progressCallback with elapsed seconds while waiting.
   * This is the primary method used by the chat panel.
   */
  async runAndPoll(
    template: string,
    prompt: string,
    model: string,
    progressCallback?: (elapsedSecs: number) => void
  ): Promise<AgentRunResult> {
    const body = JSON.stringify({
      template,
      prompt,
      model: model || this.model,
    });

    const data = await this.post("/api/agents/run", body);
    const parsed = JSON.parse(data);
    const agentId: string = parsed.agent_id;

    if (!agentId) {
      throw new Error("Daemon did not return an agent_id");
    }

    const start = Date.now();
    const timeoutMs = 300_000; // 5 minutes

    while (Date.now() - start < timeoutMs) {
      await sleep(500);

      const elapsedSecs = Math.floor((Date.now() - start) / 1000);
      progressCallback?.(elapsedSecs);

      try {
        const agentData = await this.get(`/api/agents/${agentId}`);
        const agent = JSON.parse(agentData);
        const status: string = agent.status ?? "";

        if (
          status === "Completed" ||
          status === "completed" ||
          status === "Failed" ||
          status === "failed"
        ) {
          return {
            agent_id: agentId,
            success: status === "Completed" || status === "completed",
            iterations: agent.iterations ?? 0,
            tool_invocations: agent.tool_invocations ?? 0,
            summary: agent.result_summary ?? agent.summary ?? "",
          };
        }
      } catch {
        // transient polling error — keep waiting
      }
    }

    throw new Error("Agent run timed out after 5 minutes");
  }

  async complete(req: CompletionRequest): Promise<string> {
    const body = JSON.stringify({
      prefix: req.prefix,
      suffix: req.suffix,
      language: req.language,
      model: this.model,
      max_tokens: req.maxTokens,
    });

    const t0 = Date.now();
    try {
      // Try streaming endpoint first
      const data = await this.post("/api/complete/stream", body);
      this.lastLatencyMs = Date.now() - t0;
      const tokens: string[] = [];
      for (const line of data.split("\n")) {
        if (line.startsWith("data: ")) {
          try {
            const parsed = JSON.parse(line.substring(6));
            if (parsed.text) {
              tokens.push(parsed.text);
            }
          } catch {
            // skip non-JSON data lines
          }
        }
      }
      if (tokens.length > 0) {
        return tokens.join("");
      }
      // Fallback: synchronous endpoint
      const syncData = await this.post("/api/complete", body);
      const syncParsed = JSON.parse(syncData);
      return syncParsed.completion || "";
    } catch {
      // Final fallback: run a short agent task
      return this.completeViaAgent(req);
    }
  }

  private async completeViaAgent(req: CompletionRequest): Promise<string> {
    const prompt = this.buildPrompt(req);
    try {
      const result = await this.runAndPoll("chat-assistant", prompt, this.model);
      return result.summary;
    } catch {
      return "";
    }
  }

  private buildPrompt(req: CompletionRequest): string {
    const lines = [
      `Complete the following ${req.language} code. Return ONLY the completion text, no explanation.`,
      "",
      "```" + req.language,
      req.prefix,
      "█",
    ];
    if (req.suffix) {
      lines.push(req.suffix);
    }
    lines.push("```");
    lines.push("");
    lines.push(
      `Respond with ONLY the code that replaces █. Maximum ${req.maxTokens} tokens. No markdown fences.`
    );
    return lines.join("\n");
  }

  private get(path: string): Promise<string> {
    return this.request("GET", path);
  }

  private post(path: string, body: string): Promise<string> {
    return this.request("POST", path, body);
  }

  private request(
    method: string,
    path: string,
    body?: string
  ): Promise<string> {
    return new Promise((resolve, reject) => {
      const url = new URL(path, this.endpoint);
      const isHttps = url.protocol === "https:";
      const lib = isHttps ? https : http;

      const options: http.RequestOptions = {
        hostname: url.hostname,
        port: url.port,
        path: url.pathname + url.search,
        method,
        headers: {
          "Content-Type": "application/json",
          ...(this.apiKey
            ? { Authorization: `Bearer ${this.apiKey}` }
            : {}),
          ...(body ? { "Content-Length": Buffer.byteLength(body) } : {}),
        },
        timeout: 30_000,
      };

      const req = lib.request(options, (res) => {
        let data = "";
        res.on("data", (chunk: Buffer) => (data += chunk.toString()));
        res.on("end", () => {
          if (res.statusCode && res.statusCode >= 400) {
            reject(new Error(`HTTP ${res.statusCode}: ${data}`));
          } else {
            resolve(data);
          }
        });
      });

      req.on("error", reject);
      req.on("timeout", () => {
        req.destroy();
        reject(new Error("request timeout"));
      });

      if (body) {
        req.write(body);
      }
      req.end();
    });
  }

  // ── Control-plane APIs ─────────────────────────────────────────────────────

  async listParallelRuns(): Promise<any[]> {
    try {
      const data = await this.get("/api/parallel/runs");
      const parsed = JSON.parse(data);
      return parsed.runs ?? parsed ?? [];
    } catch { return []; }
  }

  async listSwarmRuns(): Promise<any[]> {
    try {
      const data = await this.get("/api/swarm/runs");
      const parsed = JSON.parse(data);
      return parsed.runs ?? parsed ?? [];
    } catch { return []; }
  }

  async getRunConflicts(runId: string): Promise<any[]> {
    try {
      const data = await this.get(`/api/parallel/runs/${runId}/conflicts`);
      const parsed = JSON.parse(data);
      return parsed.conflicts ?? [];
    } catch { return []; }
  }

  async getAuditLog(limit = 100): Promise<any[]> {
    try {
      const data = await this.get(`/api/audit?limit=${limit}`);
      const parsed = JSON.parse(data);
      return parsed.events ?? parsed ?? [];
    } catch { return []; }
  }

  async getPolicyViolations(): Promise<any[]> {
    try {
      const data = await this.get("/api/policy");
      const parsed = JSON.parse(data);
      return parsed.violations ?? [];
    } catch { return []; }
  }

  async startSwarmRun(goal: string, files: string[], model?: string): Promise<string> {
    const body = JSON.stringify({ goal, files, planner_model: model ?? this.model });
    const data = await this.post("/api/swarm/runs", body);
    const parsed = JSON.parse(data);
    return parsed.run_id ?? "";
  }

  async validatePolicy(): Promise<any> {
    try {
      const data = await this.get("/api/policy");
      return JSON.parse(data);
    } catch (e: any) { return { error: e.message }; }
  }

  /**
   * Stream a chat prompt from the daemon.
   * Calls onToken with each new chunk of text.
   */
  async streamChat(
    prompt: string,
    model: string,
    onToken: (token: string) => void
  ): Promise<void> {
    return new Promise((resolve, reject) => {
      const url = new URL("/api/chat/stream", this.endpoint);
      const isHttps = url.protocol === "https:";
      const lib = isHttps ? https : http;

      const body = JSON.stringify({ prompt, model: model || this.model });

      const options: http.RequestOptions = {
        hostname: url.hostname,
        port: url.port,
        path: url.pathname + url.search,
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          ...(this.apiKey ? { Authorization: `Bearer ${this.apiKey}` } : {}),
        },
      };

      const req = lib.request(options, (res) => {
        if (res.statusCode && res.statusCode >= 400) {
          let errData = "";
          res.on("data", (c) => (errData += c));
          res.on("end", () => reject(new Error(`HTTP ${res.statusCode}: ${errData}`)));
          return;
        }

        res.on("data", (chunk: Buffer) => {
          const lines = chunk.toString().split("\n");
          for (const line of lines) {
            if (line.startsWith("data: ")) {
              try {
                const data = JSON.parse(line.slice(6));
                if (data.text) {
                  onToken(data.text);
                }
              } catch {
                // partial JSON or non-token data
              }
            } else if (line.startsWith("event: done")) {
                // Done
            }
          }
        });

        res.on("end", resolve);
        res.on("error", reject);
      });

      req.on("error", reject);
      req.write(body);
      req.end();
    });
  }
}

function sleep(ms: number): Promise<void> {
  return new Promise((r) => setTimeout(r, ms));
}
