import json
import hashlib
import os
import glob
import urllib.request
import urllib.error
from datetime import datetime
from typing import List, Dict, Optional

# Configuration - Adapted for Tachy Sovereign Mainframe
SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))

def find_tachy_root():
    curr = SCRIPT_DIR
    while curr != "/":
        if os.path.exists(os.path.join(curr, ".tachy")):
            return curr
        curr = os.path.dirname(curr)
    return os.path.dirname(SCRIPT_DIR)

TACHY_ROOT = find_tachy_root()
AUDIT_LOG_PATH = os.path.join(TACHY_ROOT, ".tachy", "audit.jsonl")
SESSIONS_DIR = os.path.join(TACHY_ROOT, ".tachy", "sessions")
TRAINING_POOL_DIR = os.path.join(TACHY_ROOT, ".tachy", "finetune_pool")

# API Configuration
TACHY_API_URL = os.environ.get("TACHY_API_URL", "http://localhost:8000")

# Filtering Criteria
MIN_SUCCESS_SCORE = 1.0 # 1.0 = Success without human override
MIN_REWARD_THRESHOLD = 0.85 

class TachyClient:
    """Simple HTTP client for Tachy Daemon APIs."""
    def __init__(self, base_url: str):
        self.base_url = base_url

    def _call(self, endpoint: str, method: str = "GET", data: Optional[Dict] = None):
        url = f"{self.base_url}{endpoint}"
        body = json.dumps(data).encode() if data else None
        req = urllib.request.Request(url, data=body, method=method)
        req.add_header("Content-Type", "application/json")
        try:
            with urllib.request.urlopen(req) as f:
                return json.loads(f.read().decode())
        except urllib.error.URLError as e:
            print(f"[!] API Error ({endpoint}): {e}")
            return None

    def trigger_eval(self) -> Optional[Dict]:
        return self._call("/api/eval/run", "POST", {})

    def trigger_training(self, adapter_name: str) -> Optional[Dict]:
        return self._call("/api/intel/train", "POST", {"adapter_name": adapter_name})

    def get_training_status(self) -> List[Dict]:
        return self._call("/api/intel/train/status", "GET") or []

    def promote_model(self, template: str, model: str) -> Optional[Dict]:
        return self._call("/api/governance/promote", "POST", {"template": template, "model": model})

class HashChainValidator:
    """Ensures the Forensic Logic Layer (FLL) is intact before extracting data."""
    
    @staticmethod
    def to_pascal(snake_str: str) -> str:
        return "".join(x.capitalize() for x in snake_str.split("_"))

    @staticmethod
    def get_event_content(event: Dict) -> str:
        # Replicates Tachy's AuditEvent::hash_content() logic
        kind = HashChainValidator.to_pascal(event['kind'])
        severity = HashChainValidator.to_pascal(event['severity'])
        return f"{event['timestamp']}|{event['session_id']}|{kind}|{severity}|{event['detail']}|{event.get('sequence', 0)}"

    @staticmethod
    def verify_chain(events: List[Dict]) -> bool:
        if not events:
            return False
        
        prev_hash = ""
        for i, event in enumerate(events):
            content = HashChainValidator.get_event_content(event)
            raw_input = f"{prev_hash}|{content}"
            expected_hash = hashlib.sha256(raw_input.encode()).hexdigest()
            
            if event.get("hash") != expected_hash:
                if not event.get("hash"):
                    prev_hash = ""
                    continue
                print(f"[!] FLL Integrity Failure: Chain broken at event index {i} (Seq: {event.get('sequence')})")
                return False
                
            prev_hash = event["hash"]
        return True

class DataHarvester:
    """Extracts high-reward agent trajectories into LoRA-ready JSONL format."""
    def __init__(self, output_dir: str):
        self.output_dir = output_dir
        os.makedirs(self.output_dir, exist_ok=True)
        self.client = TachyClient(TACHY_API_URL)

    def run_lifecycle(self, roles: List[str] = ["security-scanner", "code-reviewer"]):
        print(f"🚀 Tachy Sovereign Swarm Orchestrator Initialized for roles: {roles}")
        print("-" * 50)

        # 1. Verify Audit Chain (FLL Integrity)
        events = self._load_audit_events()
        if not events or not HashChainValidator.verify_chain(events):
            print("[!] FLL Integrity Failure. Aborting.")
            return
        
        print("[+] Forensic Logic Layer (FLL) Verified.")

        for role in roles:
            print(f"\n[Role: {role}] Starting optimization cycle...")
            
            # 2. Harvest Role-Specific Data
            role_events = [e for e in events if e.get('agent_id') == role or e.get('detail', '').lower().contains(role)]
            total_harvested = self._harvest_sessions(events) # In production, filter by role here
            
            if total_harvested > 0:
                adapter_name = f"gemma4:26b-ft-{role}"
                print(f"[*] Triggering Specialized Training for {role}...")
                job = self.client.trigger_training(adapter_name)
                
                if job:
                    print(f"[*] Waiting for {role} training to complete...")
                    self._wait_for_job(job['id'])
                    
                    print(f"[*] Triggering Role-Specific Evaluation...")
                    report = self.client.trigger_eval() # In production, pass role to eval
                    if report:
                        self._process_eval_report(report, role, adapter_name)
            else:
                print(f"[*] No new trajectories for {role}. Skipping.")

    def _wait_for_job(self, job_id: str):
        import time
        while True:
            status_list = self.client.get_training_status()
            curr_job = next((j for j in status_list if j['id'] == job_id), None)
            if not curr_job: break
            if curr_job['status'] == 'completed': break
            if curr_job['status'] == 'failed': raise Exception(f"Job {job_id} failed")
            time.sleep(5)

    def _process_eval_report(self, report: Dict, role: str, adapter_name: str):
        should_promote = report.get('should_promote', False)
        print(f"[Eval:{role}] Win Ratio: {report.get('tuned_wins',0)}/{report.get('total_cases',1)}")
        
        if should_promote:
            print(f"[!!!] PROMOTION MET for {role}. Hot-swapping to {adapter_name}")
            self.client.promote_model(role, adapter_name)

    def _harvest_sessions(self, events: List[Dict]) -> int:
        verified_sessions = {e['session_id'] for e in events if e['session_id'] != 'daemon'}
        total_harvested = 0
        for sid in verified_sessions:
            session_file = os.path.join(SESSIONS_DIR, f"{sid}.json")
            if os.path.exists(session_file):
                total_harvested += self._process_session_file(session_file, sid)
        return total_harvested

    def _process_eval_report(self, report: Dict):
        tuned_wins = report.get('tuned_wins', 0)
        total = report.get('total_cases', 1)
        win_ratio = tuned_wins / total
        avg_sim = report.get('avg_tuned_similarity', 0.0)
        should_promote = report.get('should_promote', False)

        print(f"[Eval] Tuned Wins: {tuned_wins}/{total} ({win_ratio*100:.1f}%)")
        print(f"[Eval] Avg Similarity Score: {avg_sim:.4f}")
        print(f"[Eval] Shadow Promotion Ready: {should_promote}")

        # Promotion Policy: Rely on daemon's threshold logic
        if should_promote:
            print("[!!!] PROMOTION CRITERIA MET. Hot-swapping model adapter...")
            res = self.client.promote_model("default", "gemma4:26b-ft-sovereign")
            if res:
                print(f"[+] Successfully promoted model: {res.get('model')}")
        else:
            print("[*] Promotion criteria not met. Continuing with current model.")

    def _load_audit_events(self) -> List[Dict]:
        events = []
        if not os.path.exists(AUDIT_LOG_PATH): return []
        with open(AUDIT_LOG_PATH, 'r') as f:
            for line in f:
                if line.strip():
                    try: events.append(json.loads(line))
                    except: pass
        return events

    def _process_session_file(self, filepath: str, session_id: str) -> int:
        try:
            with open(filepath, 'r') as f: session = json.load(f)
        except: return 0

        if not session.get('success') or session.get('human_override'): return 0

        messages = session.get('messages', [])
        assistant_msgs = [m for m in messages if m['role'] == 'assistant']
        if not assistant_msgs: return 0
        
        reward = 1.0 / len(assistant_msgs)
        if reward < 0.2: return 0

        pairs = []
        prompt = None
        for msg in messages:
            content = "\n".join([b.get('text', '') for b in msg['blocks'] if 'text' in b])
            if msg['role'] == 'user': prompt = content
            elif msg['role'] == 'assistant' and prompt:
                pairs.append({"instruction": prompt, "output": content, "metadata": {"session_id": session_id, "reward": reward}})
                prompt = None

        if pairs:
            self._write_to_pool(pairs, session_id)
            return len(pairs)
        return 0

    def _write_to_pool(self, pairs: List[Dict], session_id: str):
        ts = datetime.now().strftime("%Y%m%d_%H%M%S")
        path = os.path.join(self.output_dir, f"expert_{session_id}_{ts}.jsonl")
        with open(path, 'w') as f:
            for p in pairs: f.write(json.dumps(p) + '\n')

if __name__ == "__main__":
    harvester = DataHarvester(output_dir=TRAINING_POOL_DIR)
    harvester.run_lifecycle()
