import { useEffect, useState, useRef, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/tauri';
import { listen } from '@tauri-apps/api/event';
import { appWindow } from '@tauri-apps/api/window';
import launcherLogo from './launcher-logo.png';
import './App.css';

type Tab = 'home' | 'settings' | 'about';

interface Settings {
  textColor: string;
  bgColor: string;
  accentColor: string;
}

const DEFAULTS: Settings = {
  textColor: '#d4d4d4',
  bgColor: '#141414',
  accentColor: '#ffffff',
};

function loadSettings(): Settings {
  try {
    const raw = localStorage.getItem('qwley-settings');
    if (raw) return { ...DEFAULTS, ...JSON.parse(raw) };
  } catch {}
  return DEFAULTS;
}

function saveSettings(s: Settings) {
  localStorage.setItem('qwley-settings', JSON.stringify(s));
}

export default function App() {
  const [boot, setBoot] = useState(true);
  const [progress, setProgress] = useState(0);
  const [status, setStatus] = useState('Initializing');
  const [logs, setLogs] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const [tab, setTab] = useState<Tab>('home');
  const consoleEndRef = useRef<HTMLDivElement>(null);
  const [settings, setSettings] = useState<Settings>(loadSettings);

  const update = <K extends keyof Settings>(key: K, value: Settings[K]) => {
    setSettings((prev) => {
      const next = { ...prev, [key]: value };
      saveSettings(next);
      return next;
    });
  };

  const addLog = useCallback((msg: string) => {
    setLogs((old) => [...old, msg]);
  }, []);

  const check = async () => {
    setBusy(true);
    setStatus('Checking updates');
    try {
      await invoke('check_updates');
      setStatus('Ready');
    } catch (error) {
      addLog(`ERROR: ${String(error)}`);
      setStatus('Failed');
    } finally {
      setBusy(false);
    }
  };

  useEffect(() => {
    let remove: (() => void) | undefined;
    listen<string>('launcher-log', (event) =>
      setLogs((old) => [...old, event.payload])
    ).then((fn) => (remove = fn));

    let n = 0;
    const timer = window.setInterval(() => {
      n += 3;
      setProgress(Math.min(n, 100));
      if (n >= 100) {
        window.clearInterval(timer);
        check().finally(() => setTimeout(() => setBoot(false), 300));
      }
    }, 25);

    return () => {
      window.clearInterval(timer);
      remove?.();
    };
  }, []);

  useEffect(() => {
    consoleEndRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [logs]);

  // Apply CSS variables whenever settings change
  useEffect(() => {
    const r = document.documentElement.style;
    r.setProperty('--bg-content', settings.bgColor);
    r.setProperty('--text', settings.textColor);
    r.setProperty('--accent', settings.accentColor);
    r.setProperty('--bg', '#0c0c0c');
    r.setProperty('--bg-elevated', '#1c1c1c');
  }, [settings]);

  const inject = async () => {
    setBusy(true);
    setStatus('Injecting');
    try {
      await invoke('launch_qwley');
      setStatus('Injected');
    } catch (error) {
      setStatus('Failed');
    } finally {
      setBusy(false);
    }
  };

  const killAll = async () => {
    setBusy(true);
    setStatus('Stopping');
    try {
      await invoke('kill_roblox');
      setStatus('Idle');
      addLog('[OK] All game processes terminated.');
    } catch (error) {
      addLog(`[ERROR] ${String(error)}`);
    } finally {
      setBusy(false);
    }
  };

  const restartOverlay = async () => {
    setBusy(true);
    setStatus('Overlay');
    try {
      await invoke('start_overlay');
      setStatus('Ready');
      addLog('[OK] Overlay restarted.');
    } catch (error) {
      addLog(`[ERROR] ${String(error)}`);
    } finally {
      setBusy(false);
    }
  };

  const clearLogs = () => setLogs([]);

  // ─── BOOT ───
  if (boot) {
    return (
      <div className="window-frame" style={{ background: '#0c0c0c' }}>
        <main style={{
          flex: 1, display: 'flex', flexDirection: 'column',
          alignItems: 'center', justifyContent: 'center', gap: 18,
          background: '#0c0c0c'
        }}>
          <img src={launcherLogo} alt="" style={{
            width: 50, height: 50, borderRadius: 12, objectFit: 'cover',
            opacity: 0.7, animation: 'fadeIn 0.4s ease both'
          }} />
          <div style={{
            fontSize: 10, fontWeight: 600, color: '#3a3a3a',
            letterSpacing: '0.18em', textTransform: 'uppercase',
            animation: 'fadeIn 0.4s ease 0.08s both'
          }}>QWLEY SHADE</div>
          <div style={{
            width: 180, height: 2, borderRadius: 2, overflow: 'hidden',
            background: '#1c1c1c', animation: 'fadeIn 0.4s ease 0.12s both'
          }}>
            <div style={{
              height: '100%', borderRadius: 2,
              background: '#666',
              width: `${progress}%`, transition: 'width 0.08s linear'
            }} />
          </div>
          <span style={{
            fontSize: 9, color: '#3a3a3a', fontVariantNumeric: 'tabular-nums',
            animation: 'fadeIn 0.4s ease 0.16s both'
          }}>{progress}%</span>
        </main>
      </div>
    );
  }

  // ─── MAIN ───
  return (
    <div className="window-frame">
      {/* Titlebar */}
      <header className="titlebar"
        onMouseDown={async (e) => {
          if ((e.target as HTMLElement).closest('.window-controls')) return;
          await appWindow.startDragging();
        }}
      >
        <div className="titlebar-brand">
          <img src={launcherLogo} alt="logo" />
          <span>qwley shade</span>
        </div>
        <div className="window-controls">
          <button onClick={async () => { await appWindow.minimize(); }} aria-label="Minimize">
            <i className="icon-min" />
          </button>
          <button onClick={async () => { await appWindow.toggleMaximize(); }} aria-label="Maximize">
            <i className="icon-max" />
          </button>
          <button className="close-btn" onClick={async () => { await appWindow.close(); }} aria-label="Close">
            <i className="icon-close" />
          </button>
        </div>
      </header>

      <main className="launcher fade-in">
        {/* Content */}
        <div className="content">
          {/* Sun glow effect */}
          <div className="sun-bg">
            {/* Blurred logo background */}
            <img src={launcherLogo} alt="" className="bg-blur" />
            <div className="bg-dark" />

            {/* Light rays */}
            <div className="rays">
              <div className="ray r1" />
              <div className="ray r2" />
              <div className="ray r3" />
            </div>

            {/* Glare */}
            <div className="glare" />

            {/* Sun orb */}
            <div className="sun">
              <div className="sun-core" />
              <div className="sun-streak" />
            </div>

            {/* Horizontal streak */}
            <div className="streak" />

            {/* Lens flare dots */}
            <div className="flare f1" />
            <div className="flare f2" />
            <div className="flare f3" />
            <div className="flare f4" />

            {/* Vignette */}
            <div className="vignette" />
          </div>

          {/* Tab nav */}
          <div className="tab-nav">
            <button className={`tab-btn ${tab === 'home' ? 'active' : ''}`} onClick={() => setTab('home')}>Home</button>
            <button className={`tab-btn ${tab === 'settings' ? 'active' : ''}`} onClick={() => setTab('settings')}>Settings</button>
            <button className={`tab-btn ${tab === 'about' ? 'active' : ''}`} onClick={() => setTab('about')}>About</button>
          </div>

          {/* ─── HOME ─── */}
          {tab === 'home' && (
            <div className="home">
              <div className="home-top">
                <div className="home-title">
                  <img src={launcherLogo} alt="" />
                  <span>QWLEY SHADE</span>
                </div>
                <div className="status-pill">
                  <div className={`status-dot ${busy ? 'busy' : ''}`} />
                  {status}
                </div>
              </div>

              <div className="console-wrapper">
                <div className="console" style={{
                  fontSize: 12,
                  color: settings.textColor,
                  fontFamily: "'Consolas', monospace",
                  background: settings.bgColor,
                }}>
                  {logs.length === 0 && (
                    <div className="console-line" style={{ color: '#3a3a3a' }}>
                      Waiting for events...
                    </div>
                  )}
                  {logs.map((line, i) => {
                    let cls = '';
                    if (line.includes('[OK]')) cls = 'ok';
                    else if (line.includes('ERROR')) cls = 'error';
                    else if (line.includes('[Info]') || line.includes('[System]')) cls = 'info';
                    return (
                      <div key={i} className={`console-line ${cls}`}>
                        <span className="console-line-num">{i + 1}</span>
                        {line}
                      </div>
                    );
                  })}
                  <div ref={consoleEndRef} />
                </div>
              </div>
            </div>
          )}

          {/* ─── SETTINGS ─── */}
          {tab === 'settings' && (
            <div className="settings">
              <div className="settings-title">Settings</div>

              {/* Appearance */}
              <div className="settings-group" style={{ background: 'var(--bg-elevated)' }}>
                <div className="settings-group-label" style={{ color: settings.textColor + '55' }}>Appearance</div>

                <div className="setting-row">
                  <div className="setting-label">
                    <span>Background color</span>
                    <small>Main background</small>
                  </div>
                  <input type="color" className="color-pick" value={settings.bgColor} onChange={(e) => update('bgColor', e.target.value)} />
                </div>

                <div className="setting-row">
                  <div className="setting-label">
                    <span>Text color</span>
                    <small>Primary text</small>
                  </div>
                  <input type="color" className="color-pick" value={settings.textColor} onChange={(e) => update('textColor', e.target.value)} />
                </div>

                <div className="setting-row">
                  <div className="setting-label">
                    <span>Accent color</span>
                    <small>Highlights and active elements</small>
                  </div>
                  <input type="color" className="color-pick" value={settings.accentColor} onChange={(e) => update('accentColor', e.target.value)} />
                </div>

                </div>
              </div>
          )}

          {/* ─── ABOUT ─── */}
          {tab === 'about' && (
            <div className="about">
              <img className="about-logo" src={launcherLogo} alt="" />
              <h2>QWLEY SHADE</h2>
              <div className="about-version">v3.8</div>
              <div className="about-links">
                <a className="about-link" href="https://github.com/daniil179828/QWLEY-SHADE" target="_blank" rel="noreferrer">GitHub</a>
                <button className="about-link" onClick={check}>Check Updates</button>
              </div>
            </div>
          )}

          {/* ─── BOTTOM BAR ─── */}
          {tab === 'home' && (
            <div className="bottom-bar">
              <button className="action-btn" data-tooltip="Check Updates" onClick={check} disabled={busy}>
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
                  <polyline points="23 4 23 10 17 10" />
                  <path d="M20.49 15a9 9 0 11-2.12-9.36L23 10" />
                </svg>
              </button>

              <button className="action-btn" data-tooltip="Overlay" onClick={restartOverlay} disabled={busy}>
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
                  <rect x="2" y="3" width="20" height="14" rx="2" ry="2" />
                  <line x1="8" y1="21" x2="16" y2="21" />
                  <line x1="12" y1="17" x2="12" y2="21" />
                </svg>
              </button>

              <div className="action-separator" />

              <button className="action-btn inject-btn" data-tooltip="Inject" onClick={inject} disabled={busy}
                style={{ background: settings.accentColor }}>
                {busy ? <div className="spinner" /> : (
                  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                    <polygon points="5 3 19 12 5 21 5 3" />
                  </svg>
                )}
              </button>

              <div className="action-separator" />

              <button className="action-btn" data-tooltip="Clear" onClick={clearLogs}>
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
                  <path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z" />
                  <polyline points="14 2 14 8 20 8" />
                  <line x1="9" y1="15" x2="15" y2="15" />
                </svg>
              </button>

              <button className="action-btn danger-btn" data-tooltip="Kill All" onClick={killAll} disabled={busy}>
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
                  <circle cx="12" cy="12" r="10" />
                  <line x1="15" y1="9" x2="9" y2="15" />
                  <line x1="9" y1="9" x2="15" y2="15" />
                </svg>
              </button>
            </div>
          )}
        </div>
      </main>
    </div>
  );
}
