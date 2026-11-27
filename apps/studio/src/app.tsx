// Studio shell: sign-in (account or owner token), hash routes, live state.
import React, {useCallback, useEffect, useMemo, useReducer, useRef, useState} from 'react';
import {ApiError, createClient, type Credentials, type Me} from './api/client';
import {subscribeEvents, type ConnectionState} from './api/events';
import {initialLive, reduceLive, type LiveState} from './live';
import {AudioPanel} from './audio';
import {Studio} from './studio';
import {Settings} from './settings';

export const ROUTES = {'': 'Dashboard', studio: 'Studio', audio: 'Audio', settings: 'Settings'} as const;
export type Route = keyof typeof ROUTES;

export function parseRoute(hash: string): Route {
  const r = hash.replace(/^#\/?/, '').split(/[/?]/)[0] ?? '';
  return (r in ROUTES ? r : '') as Route;
}

function useRoute(): Route {
  const [route, setRoute] = useState(() => parseRoute(location.hash));
  useEffect(() => {
    const on = () => setRoute(parseRoute(location.hash));
    window.addEventListener('hashchange', on);
    return () => window.removeEventListener('hashchange', on);
  }, []);
  return route;
}

function Login({onToken, onLogin, busy}: {onToken: (t: string) => void; onLogin: (u: string, p: string) => void; busy: boolean}) {
  const [username, setUsername] = useState('');
  const [password, setPassword] = useState('');
  const [token, setToken] = useState('');
  return (
    <section className="login" aria-label="sign in">
      <h2>Sign in</h2>
      <form onSubmit={e => { e.preventDefault(); onLogin(username, password); }}>
        <label>Username <input autoComplete="username" value={username} onChange={e => setUsername(e.target.value)}/></label>
        <label>Password <input type="password" autoComplete="current-password" value={password} onChange={e => setPassword(e.target.value)}/></label>
        <button type="submit" disabled={busy}>Sign in</button>
      </form>
      <details>
        <summary>Connect with the admin token (setup and maintenance)</summary>
        <form onSubmit={e => { e.preventDefault(); onToken(token); }}>
          <label>Admin token <input type="password" value={token} autoComplete="off" onChange={e => setToken(e.target.value)}/></label>
          <button type="submit">Connect</button>
        </form>
      </details>
    </section>
  );
}

function LiveBar({live, conn}: {live: LiveState; conn: ConnectionState | 'off'}) {
  return (
    <section aria-label="live" data-testid="live" className="livebar">
      <span data-testid="conn">{{off: 'not connected', connecting: 'connecting…', open: 'live', closed: 'reconnecting'}[conn]}</span>
      {' '}Engine: <span data-testid="engine">{live.engine === 'connected' ? live.streamState : live.engine === 'unavailable' ? 'not responding' : 'unknown'}</span>
      {' '}Program: <span data-testid="program">{live.program ? live.program.scene_id + ' r' + live.program.revision : 'none'}</span>
      {live.lastError && <span data-testid="last-error"> Error: {live.lastError}</span>}
      {live.engineRestarted && <span data-testid="restarted"> ⚠ The engine restarted</span>}
    </section>
  );
}

export function App() {
  const [creds, setCreds] = useState<Credentials>({});
  const [me, setMe] = useState<Me | null>(null);
  const [checking, setChecking] = useState(true);
  const [message, setMessage] = useState('');
  const credsRef = useRef(creds);
  credsRef.current = creds;
  const api = useMemo(() => createClient(() => credsRef.current), []);
  const [live, dispatch] = useReducer(reduceLive, initialLive);
  const [conn, setConn] = useState<ConnectionState | 'off'>('off');
  const route = useRoute();

  // An existing session cookie signs the page in without a prompt.
  useEffect(() => {
    api.me().then(m => { setCreds({csrf: m.csrf_token}); setMe(m); }).catch(() => undefined).finally(() => setChecking(false));
  }, [api]);

  // One event stream per signed-in principal.
  const signedIn = me !== null;
  useEffect(() => {
    if (!signedIn) { setConn('off'); return; }
    return subscribeEvents({token: () => credsRef.current.token ?? '', onEvent: dispatch, onState: st => setConn(st)});
  }, [signedIn, creds.token]);

  const connectToken = useCallback(async (token: string) => {
    if (token.length < 32) return;
    setCreds({token});
    credsRef.current = {token};
    try {
      setMe(await api.me());
      setMessage('');
    } catch (e) {
      setMe(null);
      setCreds({});
      setMessage(e instanceof ApiError && e.status === 401 ? 'Wrong admin token (unauthorized)' : String(e));
    }
  }, [api]);

  async function login(username: string, password: string) {
    try {
      const r = await api.login(username, password);
      setCreds({csrf: r.csrf_token});
      credsRef.current = {csrf: r.csrf_token};
      setMe(await api.me());
      setMessage('');
    } catch (e) {
      setMessage(e instanceof ApiError && e.code === 'invalid_login' ? 'Wrong username or password' : String(e));
    }
  }

  async function logout() {
    try { await api.logout(); } catch { /* signed out locally regardless */ }
    setCreds({});
    setMe(null);
    setMessage('Signed out');
  }

  const canEdit = me?.role === 'admin';
  return (
    <main>
      <header>
        <h1>T3Box Studio</h1>
        {me && <nav className="routes" aria-label="pages">
          {(Object.keys(ROUTES) as Route[]).map(r => <a key={r} href={'#/' + r} aria-current={route === r ? 'page' : undefined}>{ROUTES[r]}</a>)}
          <span data-testid="whoami">{me.kind === 'owner' ? 'admin token' : `${me.username} (${me.role})`}</span>
          <button onClick={() => void logout()}>Sign out</button>
        </nav>}
      </header>
      {!me && !checking && <Login onToken={t => void connectToken(t)} onLogin={(u, p) => void login(u, p)} busy={false}/>}
      {me && <>
        <LiveBar live={live} conn={conn}/>
        {route === '' && <Dashboard api={api} onMessage={setMessage}/>}
        {route === 'studio' && <Studio api={api} live={live} canEdit={canEdit} onMessage={setMessage}/>}
        {route === 'audio' && <AudioPanel api={api} levels={live.audio} onMessage={setMessage}/>}
        {route === 'settings' && <Settings api={api} me={me} onMessage={setMessage}/>}
      </>}
      <pre role="status">{message}</pre>
    </main>
  );
}

function Dashboard({api, onMessage}: {api: ReturnType<typeof createClient>; onMessage: (m: string) => void}) {
  async function run(f: () => Promise<void>) { try { await f(); } catch (e) { onMessage(String(e)); } }
  return (
    <section aria-label="dashboard" data-testid="dashboard">
      <h2>Stream</h2>
      <nav>
        <button onClick={() => run(async () => { const st = await api.status(); onMessage((st.engine_restarted ? '⚠ The engine restarted (check the state again)\n' : '') + JSON.stringify(st)); })}>Status</button>
        <button onClick={() => run(async () => onMessage(JSON.stringify(await api.start())))}>Start stream</button>
        <button onClick={() => run(async () => onMessage(JSON.stringify(await api.stop())))}>Stop</button>
      </nav>
      <p>Edit scenes in Studio; microphone and HDMI audio in Audio.</p>
    </section>
  );
}
