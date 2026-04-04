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

export class TachyClient {
  private endpoint: string;
  private apiKey: string;
  private model: string;

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

  async health(): Promise<HealthResponse> {
    const data = await this.get("/health");
    return JSON.parse(data);
  }

  async complete(req: CompletionRequest): Promise<string> {
    const body = JSON.stringify({
      prefix: req.prefix,
      suffix: req.suffix,
      language: req.language,
      model: this.model,
      max_tokens: req.maxTokens,
    });

    try {
      // Use the dedicated /api/complete endpoint (synchronous, fast)
      const data = await this.post("/api/complete", body);
      const parsed = JSON.parse(data);
      return parsed.completion || "";
    } catch {
      // Fallback to agent run + poll if /api/complete isn't available
      return this.completeViaAgent(req);
    }
  }

  private async completeViaAgent(req: CompletionRequest): Promise<string> {
    const prompt = this.buildPrompt(req);
    const body = JSON.stringify({
      template: "chat-assistant",
      prompt,
      model: this.model,
    });

    const data = await this.post("/api/agents/run", body);
    const parsed = JSON.parse(data);

    if (parsed.agent_id) {
      return this.pollForResult(parsed.agent_id, 10_000);
    }
    return "";
  }

  private buildPrompt(req: CompletionRequest): string {
    const lines = [
      `Complete the following ${req.language} code. Return ONLY the completion text, no explanation.`,
      "",
      "```" + req.language,
      req.prefix,
      "█", // cursor marker
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

  private async pollForResult(
    agentId: string,
    timeoutMs: number
  ): Promise<string> {
    const start = Date.now();
    while (Date.now() - start < timeoutMs) {
      try {
        const data = await this.get(`/api/agents/${agentId}`);
        const agent = JSON.parse(data);
        if (
          agent.status === "Completed" ||
          agent.status === "completed" ||
          agent.status === "Failed" ||
          agent.status === "failed"
        ) {
          return agent.summary || "";
        }
      } catch {
        // ignore polling errors
      }
      await sleep(200);
    }
    return "";
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
        path: url.pathname,
        method,
        headers: {
          "Content-Type": "application/json",
          ...(this.apiKey
            ? { Authorization: `Bearer ${this.apiKey}` }
            : {}),
          ...(body ? { "Content-Length": Buffer.byteLength(body) } : {}),
        },
        timeout: 15_000,
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
}

function sleep(ms: number): Promise<void> {
  return new Promise((r) => setTimeout(r, ms));
}
