// Studio: sign-in (account or owner token) and live state; once signed in,
// the pages are TanStack Router routes (router.tsx) inside the Shell.
import {RouterProvider} from '@tanstack/react-router';
import {useCallback, useEffect, useMemo, useReducer, useRef, useState} from 'react';
import {ApiError, createClient, type Credentials, type Me} from './api/client';
import {subscribeEvents, type ConnectionState} from './api/events';
import {initialLive, reduceLive} from './live';
import {Login} from './login';
import {createStudioRouter} from './router';
import {StudioContext} from './studio-context';
import {ThemeSwitch} from './theme-switch';

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
  const [router] = useState(() => createStudioRouter());

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
    <main className="mx-auto flex max-w-[1280px] flex-col gap-3 p-6">
      <header className="flex flex-wrap items-center gap-3">
        <h1 className="my-2 mr-4 text-2xl font-bold tracking-tight">T3Box Studio</h1>
        {!me && <><span className="flex-1"/><ThemeSwitch/></>}
      </header>
      {!me && !checking && <Login onToken={t => void connectToken(t)} onLogin={(u, p) => void login(u, p)} busy={false}/>}
      {me && (
        <StudioContext value={{api, me, live, conn, canEdit, onMessage: setMessage, logout: () => void logout()}}>
          <RouterProvider router={router}/>
        </StudioContext>
      )}
      <pre role="status" className="text-sm text-secondary">{message}</pre>
    </main>
  );
}
