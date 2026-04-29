/* eslint-disable @typescript-eslint/no-explicit-any */
'use client';

import React, { useState, useEffect, useRef } from 'react';

export default function Dashboard() {
  const [harnesses, setHarnesses] = useState<any[]>([]);
  const [manifest, setManifest] = useState<any>(null);
  const [stats, setStats] = useState({ uptime: '0h 0m', cpu: '0%', mem: '0MB' });
  const [events, setEvents] = useState<any[]>([]);
  const [visionFeed, setVisionFeed] = useState<any[]>([]);
  const [consensusReports, setConsensusReports] = useState<any[]>([]);
  const [auditLogs, setAuditLogs] = useState<any[]>([]);
  const [selectedEvent, setSelectedEvent] = useState<any>(null);
  const [swarmCapacity, setSwarmCapacity] = useState({ current: 2, max: 8 });
  const [proposals, setProposals] = useState<any[]>([]);
  const [activeTab, setActiveTab] = useState<'mission' | 'audit' | 'evolution' | 'liquidity' | 'governance' | 'hivemind' | 'roster' | 'infrastructure' | 'swarm' | 'crisis' | 'lab' | 'diplomacy'>('mission');
  const [isSimulation, setIsSimulation] = useState(false);
  const [sentinelAlerts, setSentinelAlerts] = useState<any[]>([]);
  const [daoProposals, setDaoProposals] = useState<any[]>([
    { id: 'prop-1', title: 'Expand Investment Policy', description: 'Increase protocol exposure limit to $250k for Aave-v3.', votes_yes: 4, votes_no: 1, status: 'Pending' }
  ]);
  const [alliedSwarms, setAlliedSwarms] = useState<any[]>([
    { id: 'swarm-omega', status: 'Allied', trust: 0.98, tradeBalance: '+4200 CR', activeLinks: 12 },
    { id: 'swarm-sigma', status: 'Authenticated', trust: 0.65, tradeBalance: '-120 CR', activeLinks: 4 }
  ]);
  const [expertAdapters, setExpertAdapters] = useState<any[]>([
    { id: 'expert-ui-v2', domain: 'UI Design', base: 'Llama 3.2 3B', lift: 0.18, status: 'Ready' },
    { id: 'expert-sec-v1', domain: 'Security', base: 'Llama 3.2 11B', lift: 0.24, status: 'Active' },
    { id: 'expert-fin-v3', domain: 'DeFi Finance', base: 'Llama 3.2 3B', lift: 0.05, status: 'Training' }
  ]);
  const [activeCrisis, setActiveCrisis] = useState<any>(null);
  const [swarmNetwork, setSwarmNetwork] = useState<any[]>([
    { id: 'tachy-primary', region: 'us-east-1', lat: 40.7128, lng: -74.0060, status: 'Active', peers: 4 },
    { id: 'daemon-x12', region: 'eu-central-1', lat: 50.1109, lng: 8.6821, status: 'Active', peers: 3 },
    { id: 'daemon-y99', region: 'ap-northeast-1', lat: 35.6762, lng: 139.6503, status: 'Syncing', peers: 1 }
  ]);
  const [infrastructureNodes, setInfrastructureNodes] = useState<any[]>([
    { id: 'node-x921', provider: 'Akash', cpu: '8 vCPU', gpu: 'NVIDIA A100', ram: '32GB', cost: 0.85, status: 'Active' },
    { id: 'node-y128', provider: 'Render', cpu: '4 vCPU', gpu: 'RTX 4090', ram: '16GB', cost: 0.45, status: 'Provisioning' }
  ]);
  const [hiveMindInsights, setHiveMindInsights] = useState<any[]>([
    { category: 'Pattern', content: 'Use HSL for curated premium aesthetics in dynamic UI dashboards.', ts: '12m ago' },
    { category: 'Decision', content: 'Blocked Tornado Cash interactions by default in Sovereign Liquidity Engine.', ts: '1h ago' }
  ]);
  const [agentRoster, setAgentRoster] = useState<any[]>([
    { id: 'security-scanner', role: 'Audit', trust: 0.98, mode: 'SovereignAuto', missions: 42, rating: 4.8 },
    { id: 'liquidity-engine', role: 'Finance', trust: 0.82, mode: 'Restricted', missions: 12, rating: 4.1 }
  ]);
  const [liquidityRates, setLiquidityRates] = useState<any[]>([
    { protocol: 'Aave-v3', asset: 'USDC', apy: 0.042, depth: '1.2B', risk: 0.95 },
    { protocol: 'Compound-v3', asset: 'USDC', apy: 0.038, depth: '850M', risk: 0.92 }
  ]);
  
  const eventSourceRef = useRef<EventSource | null>(null);

  // Fetch initial data and setup SSE
  useEffect(() => {
    const fetchHarnesses = async () => {
      try {
        const res = await fetch('http://localhost:7777/api/harnesses');
        const data = await res.json();
        setHarnesses(Object.values(data));
      } catch (err) { console.error('Failed to fetch harnesses:', err); }
    };

    const fetchManifest = async () => {
      try {
        const res = await fetch('http://localhost:7777/api/repo/manifest');
        const data = await res.json();
        setManifest(data);
      } catch (err) { console.error('Failed to fetch manifest:', err); }
    };

    const fetchAuditLogs = async () => {
      try {
        const res = await fetch('http://localhost:7777/api/audit');
        const data = await res.json();
        setAuditLogs(data.events || []);
      } catch (err) { console.error('Failed to fetch audit logs:', err); }
    };

    fetchHarnesses();
    fetchManifest();
    fetchAuditLogs();

    // Setup SSE for real-time events
    const setupSSE = () => {
      if (eventSourceRef.current) eventSourceRef.current.close();
      
      const es = new EventSource('http://localhost:7777/api/events');
      eventSourceRef.current = es;

      es.onmessage = (event) => {
        try {
          const data = JSON.parse(event.data);
          
          // Handle Unified MissionEvents from SSE
          if (data.kind === 'mission_event') {
            const payload = data.payload;
            if (payload.type === 'vision_update') {
              setVisionFeed(prev => [payload, ...prev].slice(0, 5));
            } else if (payload.type === 'consensus_formed') {
              setConsensusReports(prev => [payload.report, ...prev].slice(0, 3));
            }
          } else if (data.kind === 'swarm_scale') {
            setSwarmCapacity(prev => ({ ...prev, current: data.payload.max_workers }));
          } else if (data.kind === 'optimization_proposed') {
            setProposals(prev => [data.payload, ...prev]);
          } else if (data.kind === 'evolution_applied') {
            setProposals(prev => prev.map(p => p.id === data.payload.id ? { ...p, status: 'Applied' } : p));
          }
          
          setEvents(prev => [data, ...prev].slice(0, 50));
        } catch (e) {
          console.log('Raw SSE message:', event.data);
        }
      };

      es.onerror = () => {
        console.error('SSE connection lost. Reconnecting...');
        setTimeout(setupSSE, 5000);
      };
    };

    setupSSE();

    const interval = setInterval(() => {
      setStats({
        uptime: '42h 13m',
        cpu: `${(Math.random() * 5 + 2).toFixed(1)}%`,
        mem: `${(624 + Math.random() * 16).toFixed(0)}MB`
      });
    }, 5000);

    return () => {
      if (eventSourceRef.current) eventSourceRef.current.close();
      clearInterval(interval);
    };
  }, []);

  const handleApplyEvolution = async (id: string) => {
    try {
      await fetch(`http://localhost:7777/api/evolution/apply/${id}`, { method: 'POST' });
      // Event will come back via SSE
    } catch (err) { console.error('Failed to apply evolution:', err); }
  };

  return (
    <main style={{ padding: '24px', maxWidth: '1600px', margin: '0 auto' }}>
      {/* Header */}
      <header style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '24px' }}>
        <div>
          <h1 className="neon-text-cyan" style={{ fontSize: '2rem', fontWeight: 'bold' }}>Tachy War Room</h1>
          <p className="mono" style={{ fontSize: '0.8rem', color: 'var(--text-dim)' }}>Sovereign Agentic Swarm v2.0.0-Live</p>
        </div>
        <div style={{ display: 'flex', gap: '32px', alignItems: 'center' }}>
          <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'flex-end' }}>
            <span className="mono" style={{ fontSize: '0.6rem', color: 'var(--text-dim)', marginBottom: '4px' }}>SWARM CAPACITY</span>
            <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
              <div style={{ width: '80px', height: '6px', backgroundColor: 'rgba(255,255,255,0.1)', borderRadius: '3px', overflow: 'hidden' }}>
                <div style={{ 
                  width: `${(swarmCapacity.current / swarmCapacity.max) * 100}%`, 
                  height: '100%', 
                  backgroundColor: 'var(--accent-cyan)',
                  boxShadow: '0 0 10px var(--accent-cyan)',
                  transition: 'width 0.5s ease'
                }} />
              </div>
              <span className="mono" style={{ fontSize: '0.8rem', color: 'white' }}>{swarmCapacity.current}/{swarmCapacity.max}</span>
            </div>
          </div>
          <StatBox label="CPU Usage" value={stats.cpu} />
          <StatBox label="Memory" value={stats.mem} />
          <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
            <div className="pulse" style={{ width: '8px', height: '8px', backgroundColor: 'var(--accent-cyan)', borderRadius: '50%' }}></div>
            <span className="mono" style={{ fontSize: '0.7rem', color: 'var(--accent-cyan)', textTransform: 'uppercase', letterSpacing: '0.1em' }}>Daemon Live</span>
          </div>
          <div style={{ display: 'flex', alignItems: 'center', gap: '8px', paddingLeft: '16px', borderLeft: '1px solid rgba(255,255,255,0.1)' }}>
            <div style={{ width: '8px', height: '8px', backgroundColor: 'var(--accent-red)', borderRadius: '50%', boxShadow: '0 0 8px var(--accent-red)' }}></div>
            <span className="mono" style={{ fontSize: '0.7rem', color: 'var(--accent-red)', textTransform: 'uppercase', letterSpacing: '0.1em' }}>Sentinel Shield Active</span>
          </div>
        </div>
      </header>

      {/* Tab Switcher */}
      <div style={{ display: 'flex', gap: '16px', marginBottom: '24px' }}>
        <TabButton label="Live Mission" active={activeTab === 'mission'} onClick={() => setActiveTab('mission')} />
        <TabButton label="Audit Scrubber" active={activeTab === 'audit'} onClick={() => setActiveTab('audit')} />
        <TabButton label="Evolution Center" active={activeTab === 'evolution'} onClick={() => setActiveTab('evolution')} />
        <TabButton label="Liquidity Hub" active={activeTab === 'liquidity'} onClick={() => setActiveTab('liquidity')} />
        <TabButton label="Governance DAO" active={activeTab === 'governance'} onClick={() => setActiveTab('governance')} />
        <TabButton label="Hive Mind" active={activeTab === 'hivemind'} onClick={() => setActiveTab('hivemind')} />
        <TabButton label="Agent Roster" active={activeTab === 'roster'} onClick={() => setActiveTab('roster')} />
        <TabButton label="Infrastructure Hub" active={activeTab === 'infrastructure'} onClick={() => setActiveTab('infrastructure')} />
        <TabButton label="Swarm Map" active={activeTab === 'swarm'} onClick={() => setActiveTab('swarm')} />
        <TabButton label="Crisis Center" active={activeTab === 'crisis'} onClick={() => setActiveTab('crisis')} />
        <TabButton label="Intelligence Lab" active={activeTab === 'lab'} onClick={() => setActiveTab('lab')} />
        <TabButton label="Diplomacy Hub" active={activeTab === 'diplomacy'} onClick={() => setActiveTab('diplomacy')} />
      </div>

      {/* Main Content Area */}
      <div style={{ height: '75vh' }}>
        {activeTab === 'diplomacy' && (
          <div className="glass-card" style={{ padding: '24px', height: '100%', overflowY: 'auto' }}>
            <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '32px' }}>
              <div>
                <h2 className="mono" style={{ fontSize: '0.8rem', color: 'var(--text-dim)', textTransform: 'uppercase', letterSpacing: '0.15em', marginBottom: '8px' }}>Sovereign Diplomacy Hub</h2>
                <div className="mono" style={{ fontSize: '0.7rem', color: 'var(--accent-cyan)' }}>ALLIED SWARMS: {alliedSwarms.length} | HYPER-SWARM REACH: 18 NODES | TRADE VOLUME: 4.2k CR</div>
              </div>
              <button className="glass-button" style={{ padding: '8px 16px', fontSize: '0.7rem' }}>+ DISCOVER SWARMS</button>
            </div>
            
            <div style={{ display: 'grid', gridTemplateColumns: 'repeat(2, 1fr)', gap: '24px' }}>
              {alliedSwarms.map((swarm, i) => (
                <div key={i} className="glass-card" style={{ padding: '20px', borderLeft: `4px solid ${swarm.status === 'Allied' ? 'var(--accent-cyan)' : 'var(--text-dim)'}` }}>
                  <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: '16px' }}>
                    <div style={{ display: 'flex', alignItems: 'center', gap: '12px' }}>
                      <div style={{ width: '32px', height: '32px', backgroundColor: 'rgba(0, 242, 255, 0.1)', borderRadius: '50%', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
                        <span className="mono" style={{ fontSize: '0.8rem', color: 'var(--accent-cyan)' }}>{swarm.id[0].toUpperCase()}</span>
                      </div>
                      <span className="mono" style={{ fontSize: '0.8rem', color: 'white', fontWeight: 'bold' }}>{swarm.id.toUpperCase()}</span>
                    </div>
                    <span className={`mono status-pill ${swarm.status.toLowerCase()}`} style={{ fontSize: '0.6rem' }}>{swarm.status}</span>
                  </div>
                  <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: '16px', marginBottom: '20px' }}>
                    <div className="mono" style={{ fontSize: '0.65rem', color: 'var(--text-dim)' }}>TRUST SCORE: <span style={{ color: 'var(--accent-cyan)' }}>{(swarm.trust * 100).toFixed(0)}%</span></div>
                    <div className="mono" style={{ fontSize: '0.65rem', color: 'var(--text-dim)' }}>ACTIVE LINKS: <span style={{ color: 'white' }}>{swarm.activeLinks}</span></div>
                    <div className="mono" style={{ fontSize: '0.65rem', color: 'var(--text-dim)' }}>TRADE BAL: <span style={{ color: 'var(--accent-cyan)' }}>{swarm.tradeBalance}</span></div>
                    <div className="mono" style={{ fontSize: '0.65rem', color: 'var(--text-dim)' }}>REPUTATION: <span style={{ color: 'white' }}>EXCELLENT</span></div>
                  </div>
                  <div style={{ display: 'flex', gap: '8px' }}>
                    <button className="glass-button" style={{ flex: 1, fontSize: '0.65rem' }}>SYNC MEMORIES</button>
                    <button className="glass-button" style={{ flex: 1, fontSize: '0.65rem' }}>TRADE COMPUTE</button>
                  </div>
                </div>
              ))}
            </div>
            
            <div style={{ marginTop: '48px', padding: '24px', backgroundColor: 'rgba(0, 242, 255, 0.02)', borderRadius: '12px', border: '1px dashed rgba(0, 242, 255, 0.1)' }}>
              <h3 className="mono" style={{ fontSize: '0.7rem', color: 'var(--accent-cyan)', textTransform: 'uppercase', letterSpacing: '0.1em', marginBottom: '24px' }}>Hyper-Swarm Gossip Network</h3>
              <div style={{ display: 'flex', flexDirection: 'column', gap: '12px' }}>
                <div className="mono" style={{ fontSize: '0.65rem', color: 'var(--text-dim)', padding: '8px', borderBottom: '1px solid rgba(255,255,255,0.05)' }}>
                  [10:04:22] <span style={{ color: 'var(--accent-cyan)' }}>DISCOVERY:</span> Found external swarm <span style={{ color: 'white' }}>SWARM-DELTA</span> at ap-southeast-1. Trust: 0.12 (Awaiting Auth)
                </div>
                <div className="mono" style={{ fontSize: '0.65rem', color: 'var(--text-dim)', padding: '8px', borderBottom: '1px solid rgba(255,255,255,0.05)' }}>
                  [10:01:15] <span style={{ color: 'var(--accent-cyan)' }}>TRADE:</span> Exported 12 knowledge credits (CSS-Arch) to <span style={{ color: 'white' }}>SWARM-OMEGA</span>.
                </div>
                <div className="mono" style={{ fontSize: '0.65rem', color: 'var(--text-dim)', padding: '8px' }}>
                  [09:42:01] <span style={{ color: 'var(--accent-cyan)' }}>AUTH:</span> Mutual DID handshake complete with <span style={{ color: 'white' }}>SWARM-SIGMA</span>. Trust updated to 0.65.
                </div>
              </div>
            </div>
          </div>
        )}
        {activeTab === 'lab' && (
          <div className="glass-card" style={{ padding: '24px', height: '100%', overflowY: 'auto' }}>
            <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '32px' }}>
              <div>
                <h2 className="mono" style={{ fontSize: '0.8rem', color: 'var(--text-dim)', textTransform: 'uppercase', letterSpacing: '0.15em', marginBottom: '8px' }}>Sovereign Intelligence Lab</h2>
                <div className="mono" style={{ fontSize: '0.7rem', color: 'var(--accent-cyan)' }}>ACTIVE EXPERTS: {expertAdapters.length} | AVG LIFT: +{(expertAdapters.reduce((acc, a) => acc + a.lift, 0) / expertAdapters.length * 100).toFixed(1)}%</div>
              </div>
              <button className="glass-button" style={{ padding: '8px 16px', fontSize: '0.7rem' }}>+ NEW TRAINING JOB</button>
            </div>
            
            <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: '24px' }}>
              {expertAdapters.map((expert, i) => (
                <div key={i} className="glass-card" style={{ padding: '20px', borderTop: `2px solid ${expert.status === 'Active' ? 'var(--accent-cyan)' : 'var(--text-dim)'}` }}>
                  <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: '16px' }}>
                    <span className="mono" style={{ fontSize: '0.7rem', color: 'white', fontWeight: 'bold' }}>{expert.id}</span>
                    <span className={`mono status-pill ${expert.status.toLowerCase()}`} style={{ fontSize: '0.55rem' }}>{expert.status}</span>
                  </div>
                  <div style={{ display: 'flex', flexDirection: 'column', gap: '8px', marginBottom: '20px' }}>
                    <div className="mono" style={{ fontSize: '0.65rem', color: 'var(--text-dim)' }}>DOMAIN: <span style={{ color: 'white' }}>{expert.domain}</span></div>
                    <div className="mono" style={{ fontSize: '0.65rem', color: 'var(--text-dim)' }}>BASE: <span style={{ color: 'white' }}>{expert.base}</span></div>
                  </div>
                  <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                    <div className="mono" style={{ fontSize: '1rem', color: 'var(--accent-cyan)', fontWeight: 'bold' }}>+{(expert.lift * 100).toFixed(0)}%<span style={{ fontSize: '0.6rem', color: 'var(--text-dim)', marginLeft: '4px' }}>LIFT</span></div>
                    <div style={{ display: 'flex', gap: '8px' }}>
                      <button className="glass-button" style={{ fontSize: '0.6rem', padding: '4px 8px' }}>EVAL</button>
                      <button className="glass-button" style={{ fontSize: '0.6rem', padding: '4px 8px', color: expert.status === 'Active' ? 'var(--accent-cyan)' : 'white' }}>{expert.status === 'Active' ? 'ACTIVE' : 'LOAD'}</button>
                    </div>
                  </div>
                </div>
              ))}
            </div>
            
            <div style={{ marginTop: '48px', display: 'grid', gridTemplateColumns: '1fr 1fr', gap: '32px' }}>
              <div className="glass-card" style={{ padding: '24px' }}>
                <h3 className="mono" style={{ fontSize: '0.7rem', color: 'var(--text-dim)', textTransform: 'uppercase', letterSpacing: '0.1em', marginBottom: '16px' }}>Trace Distillation Pipeline</h3>
                <div className="mono" style={{ fontSize: '0.65rem', color: 'var(--text-dim)', lineHeight: '1.6' }}>
                  <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: '8px' }}>
                    <span>SESSIONS ANALYZED:</span>
                    <span style={{ color: 'white' }}>1,422</span>
                  </div>
                  <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: '8px' }}>
                    <span>GOLD TRACES EXTRACTED:</span>
                    <span style={{ color: 'white' }}>84 (6%)</span>
                  </div>
                  <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: '16px' }}>
                    <span>CURRENT DATASET SIZE:</span>
                    <span style={{ color: 'white' }}>12.4 MB</span>
                  </div>
                  <div style={{ height: '4px', backgroundColor: 'rgba(255,255,255,0.05)', borderRadius: '2px', overflow: 'hidden' }}>
                    <div style={{ width: '85%', height: '100%', backgroundColor: 'var(--accent-cyan)' }}></div>
                  </div>
                </div>
              </div>
              
              <div className="glass-card" style={{ padding: '24px' }}>
                <h3 className="mono" style={{ fontSize: '0.7rem', color: 'var(--text-dim)', textTransform: 'uppercase', letterSpacing: '0.1em', marginBottom: '16px' }}>Training Performance (LoRA)</h3>
                <div style={{ height: '100px', display: 'flex', alignItems: 'flex-end', gap: '8px' }}>
                  {[40, 60, 35, 75, 50, 90, 85].map((h, i) => (
                    <div key={i} style={{ flex: 1, height: `${h}%`, backgroundColor: 'var(--accent-cyan)', opacity: 0.3 + (h / 100) * 0.7, borderRadius: '2px 2px 0 0' }}></div>
                  ))}
                </div>
                <div className="mono" style={{ fontSize: '0.55rem', color: 'var(--text-dim)', marginTop: '8px', textAlign: 'center' }}>LOSS CURVE — JOB: EXPERT-UI-V2</div>
              </div>
            </div>
          </div>
        )}
        {activeTab === 'crisis' && (
          <div className="glass-card" style={{ padding: '24px', height: '100%', overflowY: 'auto' }}>
            <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '32px' }}>
              <div>
                <h2 className="mono" style={{ fontSize: '0.8rem', color: 'var(--text-dim)', textTransform: 'uppercase', letterSpacing: '0.15em', marginBottom: '8px' }}>Sovereign Crisis Center</h2>
                <div className="mono" style={{ fontSize: '0.7rem', color: activeCrisis ? 'var(--accent-red)' : 'var(--accent-cyan)' }}>
                  SYSTEM STATUS: {activeCrisis ? 'RED ALERT - RECOVERY IN PROGRESS' : 'NOMINAL - MONITORING FOR ANOMALIES'}
                </div>
              </div>
              <button className="glass-button" style={{ padding: '8px 16px', fontSize: '0.7rem', color: 'var(--accent-red)', borderColor: 'var(--accent-red)' }}>MANUAL RED ALERT</button>
            </div>
            
            <div style={{ display: 'grid', gridTemplateColumns: '1fr 350px', gap: '32px' }}>
              <div>
                <h3 className="mono" style={{ fontSize: '0.7rem', color: 'var(--text-dim)', textTransform: 'uppercase', letterSpacing: '0.1em', marginBottom: '16px' }}>Anomaly Telemetry</h3>
                <div style={{ height: '250px', backgroundColor: '#050505', borderRadius: '8px', border: '1px solid rgba(255,255,255,0.05)', padding: '20px', overflow: 'hidden', position: 'relative' }}>
                  {/* Mock Telemetry Graph */}
                  <div style={{ width: '100%', height: '100%', borderBottom: '1px solid rgba(255,255,255,0.1)', borderLeft: '1px solid rgba(255,255,255,0.1)' }}>
                    <div style={{ position: 'absolute', top: '40%', left: '10%', width: '80%', height: '2px', backgroundColor: 'var(--accent-cyan)', opacity: 0.3 }}></div>
                    {activeCrisis && <div className="pulse" style={{ position: 'absolute', top: '10%', right: '20%', width: '10px', height: '10px', backgroundColor: 'var(--accent-red)', borderRadius: '50%' }}></div>}
                  </div>
                  <div className="mono" style={{ fontSize: '0.6rem', color: 'var(--text-dim)', marginTop: '8px' }}>REAL-TIME SWARM TELEMETRY (GLOBAL)</div>
                </div>
                
                {activeCrisis && (
                  <div className="glass-card" style={{ marginTop: '24px', padding: '20px', borderLeft: '4px solid var(--accent-red)' }}>
                    <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: '12px' }}>
                      <span className="mono" style={{ fontSize: '0.8rem', color: 'var(--accent-red)', fontWeight: 'bold' }}>RED ALERT: {activeCrisis.detail}</span>
                      <span className="mono" style={{ fontSize: '0.6rem', color: 'var(--text-dim)' }}>ID: {activeCrisis.id}</span>
                    </div>
                    <div style={{ display: 'flex', gap: '12px' }}>
                      <button className="glass-button" style={{ flex: 1, fontSize: '0.7rem' }}>CONFIRM RECOVERY</button>
                      <button className="glass-button" style={{ flex: 1, fontSize: '0.7rem', color: 'var(--text-dim)' }}>FALSE POSITIVE</button>
                    </div>
                  </div>
                )}
              </div>
              
              <div style={{ display: 'flex', flexDirection: 'column', gap: '20px' }}>
                <h3 className="mono" style={{ fontSize: '0.7rem', color: 'var(--text-dim)', textTransform: 'uppercase', letterSpacing: '0.1em' }}>Recovery Playbooks</h3>
                <PlaybookItem title="DE-RISK TO STABLE" description="Move all protocol assets to USDC/USDT in EmergencyVault." active={true} />
                <PlaybookItem title="EMERGENCY KILL SWITCH" description="Instantly terminate all active tool execution and lock API keys." active={false} />
                <PlaybookItem title="INFRA MIGRATION" description="Replicate daemon state to secondary cloud provider and terminate primary." active={false} />
              </div>
            </div>
          </div>
        )}
        {activeTab === 'swarm' && (
          <div className="glass-card" style={{ padding: '24px', height: '100%', overflowY: 'auto' }}>
            <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '32px' }}>
              <div>
                <h2 className="mono" style={{ fontSize: '0.8rem', color: 'var(--text-dim)', textTransform: 'uppercase', letterSpacing: '0.15em', marginBottom: '8px' }}>Global Swarm Map (Sovereign Expansion)</h2>
                <div className="mono" style={{ fontSize: '0.7rem', color: 'var(--accent-cyan)' }}>TOTAL DAEMONS: {swarmNetwork.length} | NETWORK HEALTH: 100% | P2P LINKS: {swarmNetwork.reduce((acc, n) => acc + n.peers, 0)}</div>
              </div>
              <button className="glass-button" style={{ padding: '8px 16px', fontSize: '0.7rem' }}>REPLICATE DAEMON</button>
            </div>
            
            <div style={{ height: '400px', backgroundColor: 'rgba(0, 242, 255, 0.02)', borderRadius: '12px', border: '1px solid rgba(0, 242, 255, 0.1)', position: 'relative', overflow: 'hidden' }}>
              {/* Mock 3D Globe / Map Visualization */}
              <div style={{ position: 'absolute', top: '50%', left: '50%', transform: 'translate(-50%, -50%)', width: '300px', height: '300px', border: '1px dashed rgba(255,255,255,0.1)', borderRadius: '50%' }}></div>
              {swarmNetwork.map((node, i) => (
                <div key={i} style={{ position: 'absolute', top: `${40 + i * 15}%`, left: `${30 + i * 20}%`, display: 'flex', flexDirection: 'column', alignItems: 'center' }}>
                  <div className="pulse" style={{ width: '12px', height: '12px', backgroundColor: 'var(--accent-cyan)', borderRadius: '50%', boxShadow: '0 0 12px var(--accent-cyan)' }}></div>
                  <span className="mono" style={{ fontSize: '0.6rem', color: 'white', marginTop: '4px' }}>{node.id}</span>
                  <span className="mono" style={{ fontSize: '0.5rem', color: 'var(--text-dim)' }}>{node.region}</span>
                </div>
              ))}
              <div style={{ position: 'absolute', bottom: '20px', left: '20px' }}>
                <span className="mono" style={{ fontSize: '0.6rem', color: 'var(--text-dim)' }}>LATENCY: 42ms (Primary {'->'} Japan)</span>
              </div>
            </div>
            
            <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: '24px', marginTop: '32px' }}>
              {swarmNetwork.map((node, i) => (
                <div key={i} className="glass-card" style={{ padding: '16px', backgroundColor: 'rgba(255,255,255,0.02)' }}>
                  <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: '12px' }}>
                    <span className="mono" style={{ fontSize: '0.7rem', color: 'white', fontWeight: 'bold' }}>{node.id}</span>
                    <span className={`mono status-pill ${node.status.toLowerCase()}`} style={{ fontSize: '0.55rem' }}>{node.status}</span>
                  </div>
                  <div className="mono" style={{ fontSize: '0.6rem', color: 'var(--text-dim)', marginBottom: '4px' }}>REGION: {node.region}</div>
                  <div className="mono" style={{ fontSize: '0.6rem', color: 'var(--text-dim)' }}>PEER CONNECTIONS: {node.peers}</div>
                </div>
              ))}
            </div>
          </div>
        )}
        {activeTab === 'infrastructure' && (
          <div className="glass-card" style={{ padding: '24px', height: '100%', overflowY: 'auto' }}>
            <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '32px' }}>
              <div>
                <h2 className="mono" style={{ fontSize: '0.8rem', color: 'var(--text-dim)', textTransform: 'uppercase', letterSpacing: '0.15em', marginBottom: '8px' }}>Sovereign Infrastructure Hub</h2>
                <div className="mono" style={{ fontSize: '0.7rem', color: 'var(--accent-cyan)' }}>TOTAL NODES: {infrastructureNodes.length} | HOURLY BURN: ${infrastructureNodes.reduce((acc, n) => acc + n.cost, 0).toFixed(2)}/hr</div>
              </div>
              <button className="glass-button" style={{ padding: '8px 16px', fontSize: '0.7rem' }}>+ PROVISION CLUSTER</button>
            </div>
            
            <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: '24px' }}>
              {infrastructureNodes.map((node, i) => (
                <div key={i} className="glass-card" style={{ padding: '20px', borderTop: `2px solid ${node.status === 'Active' ? 'var(--accent-cyan)' : 'var(--text-dim)'}` }}>
                  <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: '16px' }}>
                    <span className="mono" style={{ fontSize: '0.7rem', color: 'white', fontWeight: 'bold' }}>{node.id}</span>
                    <span className="mono" style={{ fontSize: '0.6rem', padding: '2px 6px', backgroundColor: 'rgba(255,255,255,0.05)', borderRadius: '4px' }}>{node.provider.toUpperCase()}</span>
                  </div>
                  <div style={{ display: 'flex', flexDirection: 'column', gap: '8px', marginBottom: '20px' }}>
                    <div className="mono" style={{ fontSize: '0.65rem', color: 'var(--text-dim)' }}>CPU: <span style={{ color: 'white' }}>{node.cpu}</span></div>
                    <div className="mono" style={{ fontSize: '0.65rem', color: 'var(--text-dim)' }}>GPU: <span style={{ color: 'white' }}>{node.gpu}</span></div>
                    <div className="mono" style={{ fontSize: '0.65rem', color: 'var(--text-dim)' }}>RAM: <span style={{ color: 'white' }}>{node.ram}</span></div>
                  </div>
                  <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                    <div className="mono" style={{ fontSize: '0.9rem', color: 'var(--accent-cyan)', fontWeight: 'bold' }}>${node.cost.toFixed(2)}<span style={{ fontSize: '0.6rem', color: 'var(--text-dim)' }}>/hr</span></div>
                    <button className="glass-button" style={{ fontSize: '0.6rem', padding: '4px 8px', color: 'var(--accent-red)' }}>TERMINATE</button>
                  </div>
                </div>
              ))}
            </div>
            
            <div style={{ marginTop: '48px', padding: '24px', backgroundColor: 'rgba(0, 242, 255, 0.02)', borderRadius: '12px', border: '1px dashed rgba(0, 242, 255, 0.1)' }}>
              <h3 className="mono" style={{ fontSize: '0.7rem', color: 'var(--accent-cyan)', textTransform: 'uppercase', letterSpacing: '0.1em', marginBottom: '16px' }}>Infrastructure Budget Enforcer</h3>
              <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                <div style={{ flex: 1, marginRight: '48px' }}>
                  <div style={{ height: '8px', backgroundColor: 'rgba(255,255,255,0.05)', borderRadius: '4px', overflow: 'hidden' }}>
                    <div style={{ width: '65%', height: '100%', backgroundColor: 'var(--accent-cyan)' }}></div>
                  </div>
                  <div style={{ display: 'flex', justifyContent: 'space-between', marginTop: '8px' }} className="mono">
                    <span style={{ fontSize: '0.6rem', color: 'var(--text-dim)' }}>SPENT: $12.45</span>
                    <span style={{ fontSize: '0.6rem', color: 'var(--text-dim)' }}>DAILY BUDGET: $20.00</span>
                  </div>
                </div>
                <button className="glass-button" style={{ padding: '8px 16px', fontSize: '0.7rem' }}>ADJUST LIMITS</button>
              </div>
            </div>
          </div>
        )}
        {activeTab === 'roster' && (
          <div className="glass-card" style={{ padding: '24px', height: '100%', overflowY: 'auto' }}>
            <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '32px' }}>
              <h2 className="mono" style={{ fontSize: '0.8rem', color: 'var(--text-dim)', textTransform: 'uppercase', letterSpacing: '0.15em' }}>Swarm Roster (Meritocratic Trust)</h2>
              <div className="mono" style={{ fontSize: '0.7rem', color: 'var(--text-dim)' }}>ACTIVE AGENTS: {agentRoster.length} | TOP PERFORMER: {agentRoster[0].id}</div>
            </div>
            
            <table style={{ width: '100%', borderCollapse: 'collapse' }}>
              <thead>
                <tr className="mono" style={{ fontSize: '0.65rem', color: 'var(--text-dim)', textAlign: 'left', borderBottom: '1px solid rgba(255,255,255,0.05)' }}>
                  <th style={{ padding: '12px' }}>AGENT IDENTITY</th>
                  <th style={{ padding: '12px' }}>ROLE</th>
                  <th style={{ padding: '12px' }}>TRUST INDEX</th>
                  <th style={{ padding: '12px' }}>PERMISSION MODE</th>
                  <th style={{ padding: '12px' }}>MISSIONS</th>
                  <th style={{ padding: '12px' }}>ACTIONS</th>
                </tr>
              </thead>
              <tbody>
                {agentRoster.map((agent, i) => (
                  <tr key={i} style={{ borderBottom: '1px solid rgba(255,255,255,0.02)' }}>
                    <td style={{ padding: '16px 12px' }}>
                      <div className="mono" style={{ fontSize: '0.85rem', color: 'white', fontWeight: 'bold' }}>{agent.id}</div>
                    </td>
                    <td style={{ padding: '16px 12px' }}>
                      <span className="mono" style={{ fontSize: '0.7rem', padding: '2px 6px', backgroundColor: 'rgba(255,255,255,0.05)', borderRadius: '4px' }}>{agent.role}</span>
                    </td>
                    <td style={{ padding: '16px 12px' }}>
                      <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
                        <div style={{ width: '60px', height: '4px', backgroundColor: 'rgba(255,255,255,0.1)', borderRadius: '2px', overflow: 'hidden' }}>
                          <div style={{ width: `${agent.trust * 100}%`, height: '100%', backgroundColor: 'var(--accent-cyan)' }}></div>
                        </div>
                        <span className="mono" style={{ fontSize: '0.75rem', color: 'var(--accent-cyan)' }}>{(agent.trust * 100).toFixed(0)}%</span>
                      </div>
                    </td>
                    <td style={{ padding: '16px 12px' }}>
                      <span className="mono" style={{ fontSize: '0.7rem', color: agent.mode === 'SovereignAuto' ? 'var(--accent-cyan)' : 'var(--text-dim)' }}>{agent.mode}</span>
                    </td>
                    <td style={{ padding: '16px 12px' }}>
                      <span className="mono" style={{ fontSize: '0.75rem', color: 'white' }}>{agent.missions}</span>
                    </td>
                    <td style={{ padding: '16px 12px' }}>
                      <div style={{ display: 'flex', gap: '8px' }}>
                        <button className="glass-button" style={{ fontSize: '0.6rem', padding: '4px 8px' }}>PROMOTE</button>
                        <button className="glass-button" style={{ fontSize: '0.6rem', padding: '4px 8px', color: 'var(--accent-red)' }}>RETIRE</button>
                      </div>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
        {activeTab === 'hivemind' && (
          <div className="glass-card" style={{ padding: '24px', height: '100%', overflowY: 'auto' }}>
            <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '32px' }}>
              <div>
                <h2 className="mono" style={{ fontSize: '0.8rem', color: 'var(--text-dim)', textTransform: 'uppercase', letterSpacing: '0.15em', marginBottom: '8px' }}>Swarm Hive Mind (Collective Intelligence)</h2>
                <div className="mono" style={{ fontSize: '0.7rem', color: 'var(--accent-cyan)' }}>TOTAL SHARED INSIGHTS: {hiveMindInsights.length} | GROWTH: +12% this week</div>
              </div>
              <button className="glass-button" style={{ padding: '8px 16px', fontSize: '0.7rem' }}>EXTRACT KNOWLEDGE</button>
            </div>
            
            <div style={{ display: 'grid', gridTemplateColumns: '1fr 300px', gap: '32px' }}>
              <div style={{ display: 'flex', flexDirection: 'column', gap: '16px' }}>
                <h3 className="mono" style={{ fontSize: '0.7rem', color: 'var(--text-dim)', textTransform: 'uppercase', letterSpacing: '0.1em' }}>Recent Global Syndications</h3>
                {hiveMindInsights.map((insight, i) => (
                  <div key={i} className="glass-card" style={{ padding: '16px', borderLeft: '2px solid var(--accent-cyan)' }}>
                    <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: '8px' }}>
                      <span className="mono" style={{ fontSize: '0.6rem', color: 'var(--accent-cyan)', textTransform: 'uppercase' }}>{insight.category}</span>
                      <span className="mono" style={{ fontSize: '0.6rem', color: 'var(--text-dim)' }}>{insight.ts}</span>
                    </div>
                    <p style={{ fontSize: '0.85rem', color: 'white', lineHeight: '1.5' }}>{insight.content}</p>
                  </div>
                ))}
              </div>
              
              <div className="glass-card" style={{ padding: '20px', backgroundColor: 'rgba(255,255,255,0.02)' }}>
                <h3 className="mono" style={{ fontSize: '0.7rem', color: 'var(--text-dim)', textTransform: 'uppercase', letterSpacing: '0.1em', marginBottom: '16px' }}>Intelligence Distribution</h3>
                <div style={{ height: '150px', display: 'flex', alignItems: 'flex-end', gap: '12px', paddingBottom: '20px', borderBottom: '1px solid rgba(255,255,255,0.1)' }}>
                  <div style={{ flex: 1, height: '60%', backgroundColor: 'var(--accent-cyan)', borderRadius: '2px 2px 0 0' }}></div>
                  <div style={{ flex: 1, height: '40%', backgroundColor: 'var(--accent-cyan)', opacity: 0.6, borderRadius: '2px 2px 0 0' }}></div>
                  <div style={{ flex: 1, height: '80%', backgroundColor: 'var(--accent-cyan)', opacity: 0.8, borderRadius: '2px 2px 0 0' }}></div>
                  <div style={{ flex: 1, height: '30%', backgroundColor: 'var(--accent-cyan)', opacity: 0.4, borderRadius: '2px 2px 0 0' }}></div>
                </div>
                <div style={{ display: 'flex', justifyContent: 'space-between', marginTop: '8px' }} className="mono">
                  <span style={{ fontSize: '0.6rem', color: 'var(--text-dim)' }}>MON</span>
                  <span style={{ fontSize: '0.6rem', color: 'var(--text-dim)' }}>WED</span>
                  <span style={{ fontSize: '0.6rem', color: 'var(--text-dim)' }}>FRI</span>
                  <span style={{ fontSize: '0.6rem', color: 'var(--text-dim)' }}>SUN</span>
                </div>
                <div style={{ marginTop: '24px' }}>
                  <p className="mono" style={{ fontSize: '0.65rem', color: 'var(--text-dim)', lineHeight: '1.4' }}>
                    Hive Mind identifies <strong>CSS Architecture</strong> as the highest-growth knowledge area.
                  </p>
                </div>
              </div>
            </div>
          </div>
        )}
        {activeTab === 'governance' && (
          <div className="glass-card" style={{ padding: '24px', height: '100%', overflowY: 'auto' }}>
            <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '24px' }}>
              <h2 className="mono" style={{ fontSize: '0.8rem', color: 'var(--text-dim)', textTransform: 'uppercase', letterSpacing: '0.15em' }}>Sovereign Governance DAO</h2>
              <button className="glass-button" style={{ padding: '8px 16px', fontSize: '0.7rem' }}>+ NEW PROPOSAL</button>
            </div>
            <div style={{ display: 'flex', flexDirection: 'column', gap: '16px' }}>
              {daoProposals.map((prop, i) => (
                <div key={i} className="glass-card" style={{ padding: '20px', display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                  <div style={{ maxWidth: '60%' }}>
                    <div style={{ display: 'flex', alignItems: 'center', gap: '12px', marginBottom: '8px' }}>
                      <span className="mono" style={{ fontSize: '0.6rem', padding: '2px 6px', backgroundColor: 'var(--accent-cyan)', color: 'black', borderRadius: '4px' }}>{prop.id}</span>
                      <h3 style={{ fontSize: '1rem', fontWeight: 'bold', color: 'white' }}>{prop.title}</h3>
                    </div>
                    <p style={{ fontSize: '0.8rem', color: 'var(--text-dim)' }}>{prop.description}</p>
                  </div>
                  <div style={{ textAlign: 'right' }}>
                    <div className="mono" style={{ fontSize: '0.7rem', color: 'var(--text-dim)', marginBottom: '8px' }}>
                      YES: <span style={{ color: 'var(--accent-cyan)' }}>{prop.votes_yes}</span> | NO: <span style={{ color: 'var(--accent-red)' }}>{prop.votes_no}</span>
                    </div>
                    <div style={{ display: 'flex', gap: '8px' }}>
                      <button className="glass-button" style={{ color: 'var(--accent-cyan)', border: '1px solid var(--accent-cyan)' }}>VOTE YES</button>
                      <button className="glass-button" style={{ color: 'var(--accent-red)', border: '1px solid var(--accent-red)' }}>EXECUTIVE VETO</button>
                    </div>
                  </div>
                </div>
              ))}
            </div>
            <div style={{ marginTop: '48px', opacity: 0.5 }}>
              <h3 className="mono" style={{ fontSize: '0.7rem', color: 'var(--text-dim)', textTransform: 'uppercase', letterSpacing: '0.1em', marginBottom: '16px' }}>Sovereign Charter Rules</h3>
              <ul className="mono" style={{ fontSize: '0.7rem', color: 'var(--text-dim)', listStyle: 'none', padding: 0 }}>
                <li>- Never disable the Compliance Sentinel.</li>
                <li>- Human Veto is always absolute.</li>
                <li>- Minimum security severity for core updates is CRITICAL.</li>
              </ul>
            </div>
          </div>
        )}
        {activeTab === 'liquidity' && (
          <div className="glass-card" style={{ padding: '24px', height: '100%', overflowY: 'auto' }}>
            <h2 className="mono" style={{ fontSize: '0.8rem', color: 'var(--text-dim)', textTransform: 'uppercase', letterSpacing: '0.15em', marginBottom: '24px' }}>Sovereign Liquidity Monitoring</h2>
            <div style={{ display: 'grid', gridTemplateColumns: 'repeat(4, 1fr)', gap: '24px' }}>
              {liquidityRates.map((rate, i) => (
                <div key={i} className="glass-card" style={{ padding: '20px', borderTop: '2px solid var(--accent-cyan)' }}>
                  <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: '12px' }}>
                    <span className="mono" style={{ fontSize: '0.9rem', color: 'white' }}>{rate.protocol}</span>
                    <span className="mono" style={{ fontSize: '0.7rem', color: 'var(--accent-cyan)' }}>{rate.asset}</span>
                  </div>
                  <div style={{ fontSize: '1.5rem', fontWeight: 'bold', color: 'var(--accent-cyan)', marginBottom: '8px' }}>{(rate.apy * 100).toFixed(2)}% <span style={{ fontSize: '0.6rem', color: 'var(--text-dim)' }}>APY</span></div>
                  <div className="mono" style={{ fontSize: '0.7rem', color: 'var(--text-dim)' }}>DEPTH: {rate.depth}</div>
                  <div className="mono" style={{ fontSize: '0.7rem', color: 'var(--text-dim)', marginTop: '4px' }}>RISK SCORE: {rate.risk}</div>
                </div>
              ))}
            </div>
            <div style={{ marginTop: '48px' }}>
              <h3 className="mono" style={{ fontSize: '0.7rem', color: 'var(--text-dim)', textTransform: 'uppercase', letterSpacing: '0.1em', marginBottom: '16px' }}>Active Yield Strategies</h3>
              <div style={{ padding: '16px', border: '1px dashed rgba(255,255,255,0.1)', borderRadius: '8px', textAlign: 'center' }}>
                <p className="mono" style={{ fontSize: '0.8rem', color: 'var(--text-dim)' }}>Strategy Engine is monitoring. No rebalancing required at 0.5% threshold.</p>
              </div>
            </div>
          </div>
        )}
        {activeTab === 'mission' && (
          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(12, 1fr)', gap: '24px', height: '100%' }}>
            {/* Left Column: Vision Feed & Manifest */}
            <section style={{ gridColumn: 'span 3', display: 'flex', flexDirection: 'column', gap: '24px', overflowY: 'hidden' }}>
              <div className="glass-card" style={{ padding: '16px', flex: 1, overflowY: 'auto' }}>
                <h2 className="mono" style={{ fontSize: '0.7rem', color: 'var(--text-dim)', textTransform: 'uppercase', letterSpacing: '0.15em', marginBottom: '16px' }}>Live Vision Feed</h2>
                <div style={{ display: 'flex', flexDirection: 'column', gap: '16px' }}>
                  {visionFeed.length > 0 ? visionFeed.map((v, i) => (
                    <VisionCard key={i} agentId={v.agent_id} url={`http://localhost:7777${v.thumbnail_url}`} />
                  )) : (
                    <p className="mono" style={{ fontSize: '10px', color: 'var(--text-dim)' }}>Waiting for vision assets...</p>
                  )}
                </div>
              </div>
            </section>

            {/* Center Column: Main Loop & Consensus */}
            <section className="glass-card" style={{ gridColumn: 'span 6', padding: '16px', display: 'flex', flexDirection: 'column' }}>
              <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '16px' }}>
                <h2 className="mono" style={{ fontSize: '0.7rem', color: 'var(--text-dim)', textTransform: 'uppercase', letterSpacing: '0.15em' }}>Agentic Intelligence Trace</h2>
                <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
                  <span className="mono" style={{ fontSize: '0.6rem', color: isSimulation ? 'var(--accent-cyan)' : 'var(--text-dim)' }}>{isSimulation ? 'SIMULATION MODE' : 'LIVE MISSION'}</span>
                  <button 
                    onClick={() => setIsSimulation(!isSimulation)}
                    style={{ 
                      width: '32px', height: '16px', borderRadius: '8px', 
                      backgroundColor: isSimulation ? 'var(--accent-cyan)' : 'rgba(255,255,255,0.1)', 
                      position: 'relative', border: 'none', cursor: 'pointer', transition: 'all 0.3s ease'
                    }}
                  >
                    <div style={{ 
                      width: '12px', height: '12px', borderRadius: '50%', backgroundColor: 'white',
                      position: 'absolute', top: '2px', left: isSimulation ? '18px' : '2px', transition: 'all 0.3s ease'
                    }} />
                  </button>
                </div>
              </div>
              <div style={{ flex: 1, display: 'flex', flexDirection: 'column', gap: '20px', overflowY: 'auto', paddingRight: '8px' }}>
                <div style={{ display: 'flex', flexDirection: 'column', gap: '16px' }}>
                  <TraceStep name="Swarm Planning" status="COMPLETE" detail="Generated multi-agent execution plan for Phase 31." />
                  <TraceStep name="Self-Evolution" status="ACTIVE" detail="Analyzing mission logs for intelligence optimization." />
                </div>

                <div style={{ marginTop: '24px', borderTop: '1px solid rgba(255,255,255,0.05)', paddingTop: '20px' }}>
                  <h2 className="mono" style={{ fontSize: '0.65rem', color: 'var(--text-dim)', textTransform: 'uppercase', letterSpacing: '0.15em', marginBottom: '16px' }}>Governance Consensus Reports</h2>
                  <div style={{ display: 'flex', flexDirection: 'column', gap: '16px' }}>
                    {consensusReports.length > 0 ? consensusReports.map((report, i) => (
                      <ConsensusCard key={i} report={report} />
                    )) : (
                      <div style={{ padding: '12px', border: '1px dashed rgba(255,255,255,0.1)', borderRadius: '8px', textAlign: 'center' }}>
                        <p className="mono" style={{ fontSize: '10px', color: 'var(--text-dim)' }}>Awaiting swarm consensus events...</p>
                      </div>
                    )}
                  </div>
                </div>
              </div>
            </section>

            {/* Right Column: Mission Feed */}
            <section className="glass-card" style={{ gridColumn: 'span 3', padding: '16px', display: 'flex', flexDirection: 'column', overflowY: 'hidden' }}>
              <h2 className="mono" style={{ fontSize: '0.7rem', color: 'var(--text-dim)', textTransform: 'uppercase', letterSpacing: '0.15em', marginBottom: '16px' }}>Mission Timeline</h2>
              <div style={{ flex: 1, overflowY: 'auto', display: 'flex', flexDirection: 'column', gap: '8px' }}>
                {events.map((e, i) => (
                  <div key={i} style={{ padding: '8px', borderBottom: '1px solid rgba(255,255,255,0.05)' }}>
                    <span className="mono" style={{ fontSize: '8px', color: 'var(--text-dim)' }}>{new Date().toLocaleTimeString()}</span>
                    <p className="mono" style={{ fontSize: '9px', margin: '4px 0', color: e.kind === 'swarm_scale' ? 'var(--accent-cyan)' : 'white' }}>
                      {e.payload?.message || e.detail || JSON.stringify(e.payload)}
                    </p>
                  </div>
                ))}
              </div>
            </section>
          </div>
        )}

        {activeTab === 'audit' && (
          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(12, 1fr)', gap: '24px', height: '100%' }}>
            <section className="glass-card" style={{ gridColumn: 'span 8', padding: '24px', overflowY: 'auto' }}>
              <h2 className="mono" style={{ fontSize: '0.8rem', color: 'var(--text-dim)', textTransform: 'uppercase', letterSpacing: '0.15em', marginBottom: '24px' }}>Historical Audit Trail</h2>
              <div style={{ display: 'flex', flexDirection: 'column', gap: '12px' }}>
                {auditLogs.length > 0 ? [...auditLogs].reverse().map((e, i) => (
                  <AuditEventCard 
                    key={e.hash || i} 
                    event={e} 
                    isSelected={selectedEvent?.hash === e.hash}
                    onClick={() => setSelectedEvent(e)}
                  />
                )) : (
                  <p className="mono" style={{ fontSize: '10px', color: 'var(--text-dim)' }}>Loading audit trail...</p>
                )}
              </div>
            </section>
            <section style={{ gridColumn: 'span 4' }}>
              {selectedEvent && (
                <div className="glass-card" style={{ padding: '24px', position: 'sticky', top: 0 }}>
                  <h2 className="mono" style={{ fontSize: '0.7rem', color: 'var(--accent-cyan)', marginBottom: '16px' }}>Event Inspection</h2>
                  <p className="mono" style={{ fontSize: '11px', lineHeight: '1.6', marginBottom: '20px' }}>{selectedEvent.detail}</p>
                  <div style={{ display: 'flex', flexDirection: 'column', gap: '8px', borderTop: '1px solid rgba(255,255,255,0.1)', paddingTop: '16px' }}>
                    <span className="mono" style={{ fontSize: '9px', opacity: 0.5 }}>HASH: {selectedEvent.hash}</span>
                    <span className="mono" style={{ fontSize: '9px', opacity: 0.5 }}>AGENT: {selectedEvent.agent_id || 'SYSTEM'}</span>
                    {selectedEvent.visual_anchor && (
                       <div style={{ marginTop: '16px' }}>
                         <span className="mono" style={{ fontSize: '9px', color: 'var(--text-dim)', display: 'block', marginBottom: '8px' }}>VISUAL ANCHOR</span>
                         <img 
                           src={`http://localhost:7777/api/vision/snapshot/${selectedEvent.visual_anchor.split('/').pop()}`} 
                           style={{ width: '100%', borderRadius: '4px', border: '1px solid rgba(255,255,255,0.1)' }} 
                         />
                       </div>
                    )}
                  </div>
                </div>
              )}
            </section>
          </div>
        )}

        {activeTab === 'evolution' && (
          <div style={{ display: 'flex', flexDirection: 'column', gap: '24px', height: '100%' }}>
            <div className="glass-card" style={{ padding: '24px', flex: 1, overflowY: 'auto' }}>
              <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '24px' }}>
                <h2 className="mono" style={{ fontSize: '0.8rem', color: 'var(--text-dim)', textTransform: 'uppercase', letterSpacing: '0.15em' }}>Autonomous Evolution Center</h2>
                <div style={{ display: 'flex', alignItems: 'center', gap: '12px' }}>
                  <span className="mono" style={{ fontSize: '10px', color: 'var(--text-dim)' }}>REFINEMENT QUEUE: {proposals.length}</span>
                  <button onClick={() => fetch('http://localhost:7777/api/evolution/trigger', { method: 'POST' })} className="mono" style={{ backgroundColor: 'rgba(255,255,255,0.05)', border: '1px solid rgba(255,255,255,0.1)', color: 'white', padding: '6px 12px', fontSize: '9px', borderRadius: '4px', cursor: 'pointer' }}>TRIGGER ANALYSIS</button>
                </div>
              </div>
              <div style={{ display: 'grid', gridTemplateColumns: 'repeat(2, 1fr)', gap: '24px' }}>
                {proposals.length > 0 ? proposals.map((p, i) => (
                  <ProposalCard key={i} proposal={p} onApply={handleApplyEvolution} />
                )) : (
                  <div style={{ gridColumn: 'span 2', padding: '48px', textAlign: 'center', border: '1px dashed rgba(255,255,255,0.1)', borderRadius: '12px' }}>
                    <p className="mono" style={{ fontSize: '12px', color: 'var(--text-dim)' }}>Swarm intelligence is currently optimal. No refinements proposed.</p>
                  </div>
                )}
              </div>
            </div>
          </div>
        )}
      </div>
    </main>
  );
}

function AuditEventCard({ event, isSelected, onClick }: { event: any, isSelected: boolean, onClick: () => void }) {
  const isCritical = event.severity === 'critical';
  const isWarning = event.severity === 'warning';
  
  return (
    <div 
      onClick={onClick}
      style={{ 
        padding: '10px', 
        backgroundColor: isSelected ? 'rgba(0, 242, 255, 0.1)' : 'rgba(255,255,255,0.02)', 
        borderRadius: '6px', 
        borderLeft: `3px solid ${isCritical ? 'var(--accent-red)' : (isWarning ? '#ffaa00' : 'var(--accent-cyan)')}`,
        cursor: 'pointer',
        transition: 'all 0.2s ease'
      }}
    >
      <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: '4px' }}>
        <span className="mono" style={{ fontSize: '9px', color: isCritical ? 'var(--accent-red)' : 'var(--accent-cyan)' }}>
          {event.kind.toUpperCase()}
        </span>
        <span className="mono" style={{ fontSize: '8px', opacity: 0.4 }}>{event.timestamp}</span>
      </div>
      <p className="mono" style={{ fontSize: '10px', color: 'rgba(255,255,255,0.7)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
        {event.detail}
      </p>
    </div>
  );
}

function VisualDiff({ beforeUrl, afterUrl }: { beforeUrl: string, afterUrl: string }) {
  return (
    <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: '12px', marginTop: '16px' }}>
      <div>
        <span className="mono" style={{ fontSize: '8px', color: 'var(--text-dim)', textTransform: 'uppercase' }}>Design Intent (Mock)</span>
        <div style={{ aspectRatio: '16/9', backgroundColor: '#050505', borderRadius: '4px', border: '1px solid rgba(255,255,255,0.05)', display: 'flex', alignItems: 'center', justifyContent: 'center', overflow: 'hidden' }}>
          <img src={beforeUrl} style={{ width: '100%', height: '100%', objectFit: 'contain', opacity: 0.5 }} />
        </div>
      </div>
      <div>
        <span className="mono" style={{ fontSize: '8px', color: 'var(--accent-cyan)', textTransform: 'uppercase' }}>Live Result (Audit)</span>
        <div style={{ aspectRatio: '16/9', backgroundColor: '#050505', borderRadius: '4px', border: '1px solid var(--accent-cyan)', display: 'flex', alignItems: 'center', justifyContent: 'center', overflow: 'hidden' }}>
          <img src={afterUrl} style={{ width: '100%', height: '100%', objectFit: 'contain' }} />
        </div>
      </div>
    </div>
  );
}

function StatBox({ label, value }: { label: string, value: string }) {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'flex-end' }}>
      <span className="mono" style={{ fontSize: '10px', textTransform: 'uppercase', letterSpacing: '0.1em', color: 'var(--text-dim)' }}>{label}</span>
      <span style={{ fontSize: '1.125rem', fontWeight: '600' }}>{value}</span>
    </div>
  );
}

function TraceStep({ name, status, detail }: { name: string, status: 'ACTIVE' | 'COMPLETE' | 'PENDING', detail: string }) {
  const isActive = status === 'ACTIVE';
  const isComplete = status === 'COMPLETE';
  return (
    <div style={{ display: 'flex', gap: '16px', opacity: status === 'PENDING' ? 0.3 : 1 }}>
      <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center' }}>
        <div style={{ 
          width: '10px', height: '10px', borderRadius: '50%', 
          backgroundColor: isActive ? 'var(--accent-cyan)' : (isComplete ? 'var(--accent-cyan)' : 'transparent'),
          border: '2px solid var(--accent-cyan)',
          boxShadow: isActive ? '0 0 8px var(--accent-cyan)' : 'none'
        }}></div>
        <div style={{ width: '1px', flex: 1, backgroundColor: 'rgba(255,255,255,0.1)', margin: '4px 0' }}></div>
      </div>
      <div style={{ paddingBottom: '8px' }}>
        <h3 className="mono" style={{ fontSize: '0.75rem', color: isActive ? 'var(--accent-cyan)' : 'white', marginBottom: '2px' }}>{name}</h3>
        <p style={{ fontSize: '0.7rem', color: 'var(--text-dim)', lineHeight: '1.4' }}>{detail}</p>
      </div>
    </div>
  );
}

function VisionCard({ agentId, url }: { agentId: string, url: string }) {
  return (
    <div style={{ position: 'relative', borderRadius: '8px', overflow: 'hidden', border: '1px solid rgba(255,255,255,0.1)' }}>
      <img src={url} alt="Vision Snapshot" style={{ width: '100%', height: 'auto', display: 'block' }} />
      <div style={{ position: 'absolute', bottom: 0, left: 0, right: 0, padding: '8px', background: 'linear-gradient(transparent, rgba(0,0,0,0.8))', display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
        <span className="mono" style={{ fontSize: '9px', color: 'var(--accent-cyan)' }}>{agentId}</span>
        <span className="mono" style={{ fontSize: '8px', color: 'white', opacity: 0.6 }}>LIVE</span>
      </div>
    </div>
  );
}

function ConsensusCard({ report }: { report: any }) {
  return (
    <div style={{ backgroundColor: 'rgba(255,255,255,0.02)', borderRadius: '8px', border: '1px solid rgba(255,255,255,0.05)', padding: '12px' }}>
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '12px' }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
          <div style={{ width: '6px', height: '6px', borderRadius: '50%', backgroundColor: report.is_approved ? 'var(--accent-cyan)' : 'var(--accent-red)' }}></div>
          <span className="mono" style={{ fontSize: '10px', fontWeight: 'bold' }}>Swarm Consensus</span>
        </div>
        <span className="mono" style={{ fontSize: '10px', color: report.is_approved ? 'var(--accent-cyan)' : 'var(--accent-red)' }}>
          SCORE: {(report.aggregate_score * 100).toFixed(0)}%
        </span>
      </div>
      <div style={{ display: 'flex', flexDirection: 'column', gap: '8px' }}>
        {report.reviews.map((r: any, j: number) => (
          <div key={j} style={{ display: 'flex', justifyContent: 'space-between', fontSize: '9px' }}>
            <span className="mono" style={{ color: 'var(--text-dim)' }}>{r.judge_name}</span>
            <div style={{ display: 'flex', gap: '8px' }}>
              <span style={{ color: r.veto ? 'var(--accent-red)' : 'var(--accent-cyan)' }}>{r.veto ? 'VETO' : 'OK'}</span>
              <span className="mono" style={{ opacity: 0.5 }}>{r.score.toFixed(2)}</span>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}

function TabButton({ label, active, onClick }: { label: string, active: boolean, onClick: () => void }) {
  return (
    <button 
      onClick={onClick}
      className="mono"
      style={{
        padding: '8px 16px',
        backgroundColor: active ? 'var(--accent-cyan)' : 'rgba(255,255,255,0.05)',
        color: active ? 'black' : 'white',
        border: 'none',
        borderRadius: '4px',
        fontSize: '0.7rem',
        cursor: 'pointer',
        transition: 'all 0.2s ease',
        fontWeight: active ? 'bold' : 'normal',
        textTransform: 'uppercase',
        letterSpacing: '0.05em'
      }}
    >
      {label}
    </button>
  );
}

function ProposalCard({ proposal, onApply }: { proposal: any, onApply: (id: string) => void }) {
  return (
    <div className="glass-card" style={{ padding: '16px', display: 'flex', flexDirection: 'column', gap: '12px' }}>
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
        <span className="mono" style={{ fontSize: '10px', color: 'var(--accent-cyan)' }}>{proposal.id}</span>
        <span className="mono" style={{ fontSize: '10px', color: 'var(--text-dim)' }}>AGENT: {proposal.template_name}</span>
      </div>
      <div>
        <h4 className="mono" style={{ fontSize: '0.8rem', marginBottom: '4px' }}>Self-Optimization Proposal</h4>
        <p style={{ fontSize: '0.75rem', color: 'rgba(255,255,255,0.7)', lineHeight: '1.4' }}>{proposal.rationale}</p>
      </div>
      <div style={{ backgroundColor: 'rgba(0,0,0,0.3)', padding: '12px', borderRadius: '4px', border: '1px solid rgba(255,255,255,0.05)' }}>
        <span className="mono" style={{ fontSize: '9px', color: 'var(--text-dim)', display: 'block', marginBottom: '8px' }}>PROPOSED REFINEMENT</span>
        <pre className="mono" style={{ fontSize: '10px', color: 'var(--accent-cyan)', margin: 0, whiteSpace: 'pre-wrap' }}>{proposal.optimized_prompt}</pre>
      </div>
      <div style={{ display: 'flex', justifyContent: 'flex-end', gap: '12px', marginTop: '8px' }}>
        <button 
          className="mono" 
          disabled={proposal.status !== 'Pending'}
          onClick={() => onApply(proposal.id)}
          style={{ 
            padding: '6px 12px', 
            fontSize: '9px', 
            backgroundColor: proposal.status === 'Applied' ? 'transparent' : 'var(--accent-cyan)',
            color: proposal.status === 'Applied' ? 'var(--accent-cyan)' : 'black',
            border: proposal.status === 'Applied' ? '1px solid var(--accent-cyan)' : 'none',
            borderRadius: '4px',
            cursor: proposal.status === 'Pending' ? 'pointer' : 'default',
            opacity: proposal.status === 'Pending' ? 1 : 0.6
          }}
        >
          {proposal.status === 'Applied' ? 'EVOLUTION APPLIED' : 'AUTHORIZE EVOLUTION'}
        </button>
      </div>
    </div>
  );
}

function PlaybookItem({ title, description, active }: { title: string, description: string, active: boolean }) {
  return (
    <div className="glass-card" style={{ padding: '16px', opacity: active ? 1 : 0.5, borderLeft: active ? '3px solid var(--accent-cyan)' : '1px solid rgba(255,255,255,0.05)' }}>
      <div className="mono" style={{ fontSize: '0.75rem', color: 'white', fontWeight: 'bold', marginBottom: '4px' }}>{title}</div>
      <p className="mono" style={{ fontSize: '0.65rem', color: 'var(--text-dim)', lineHeight: '1.4' }}>{description}</p>
    </div>
  );
}
