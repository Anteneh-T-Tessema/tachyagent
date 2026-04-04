import * as http from 'http';

export interface HealthResponse {
    status: string;
    models: number;
    agents: number;
    tasks: number;
}

export interface AgentResponse {
    id: string;
    template: string;
    status: string;
    iterations: number;
    tool_invocations: number;
    summary: string | null;
}

export interface StartAgentResponse {
    agent_id: string;
    status: string;
    message: string;
}

export class TachyClient {
    constructor(private baseUrl: string) {}

    async health(): Promise<HealthResponse | null> {
        try {
            return await this.get<HealthResponse>('/health');
        } catch {
            return null;
        }
    }

    async startAgent(template: string, prompt: string, model: string): Promise<StartAgentResponse> {
        return this.post<StartAgentResponse>('/api/agents/run', { template, prompt, model });
    }

    async getAgent(agentId: string): Promise<AgentResponse> {
        return this.get<AgentResponse>(`/api/agents/${agentId}`);
    }

    async runAndPoll(template: string, prompt: string, model: string, onProgress?: (secs: number) => void): Promise<AgentResponse> {
        const start = await this.startAgent(template, prompt, model);
        const agentId = start.agent_id;

        let elapsed = 0;
        const pollInterval = 1000;
        const maxWait = 300000;

        while (elapsed < maxWait) {
            await sleep(pollInterval);
            elapsed += pollInterval;
            if (onProgress) { onProgress(Math.round(elapsed / 1000)); }

            try {
                const agent = await this.getAgent(agentId);
                if (agent.status === 'completed' || agent.status === 'failed') {
                    return agent;
                }
            } catch {
                // Keep polling
            }
        }

        return { id: agentId, template, status: 'timeout', iterations: 0, tool_invocations: 0, summary: 'Request timed out after 5 minutes.' };
    }

    private get<T>(path: string): Promise<T> {
        return new Promise((resolve, reject) => {
            const url = new URL(path, this.baseUrl);
            http.get(url.toString(), (res) => {
                let data = '';
                res.on('data', chunk => data += chunk);
                res.on('end', () => {
                    try { resolve(JSON.parse(data)); }
                    catch (e) { reject(e); }
                });
            }).on('error', reject);
        });
    }

    private post<T>(path: string, body: object): Promise<T> {
        return new Promise((resolve, reject) => {
            const url = new URL(path, this.baseUrl);
            const payload = JSON.stringify(body);
            const req = http.request(url.toString(), {
                method: 'POST',
                headers: { 'Content-Type': 'application/json', 'Content-Length': Buffer.byteLength(payload) },
            }, (res) => {
                let data = '';
                res.on('data', chunk => data += chunk);
                res.on('end', () => {
                    try { resolve(JSON.parse(data)); }
                    catch (e) { reject(e); }
                });
            });
            req.on('error', reject);
            req.write(payload);
            req.end();
        });
    }
}

function sleep(ms: number): Promise<void> {
    return new Promise(resolve => setTimeout(resolve, ms));
}
