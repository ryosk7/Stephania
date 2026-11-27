// Sign-in: a Studio account, or the admin token for setup and maintenance.
import {LogIn} from 'lucide-react';
import {useState} from 'react';

export function Login({onToken, onLogin, busy}: {onToken: (t: string) => void; onLogin: (u: string, p: string) => void; busy: boolean}) {
  const [username, setUsername] = useState('');
  const [password, setPassword] = useState('');
  const [token, setToken] = useState('');
  return (
    <section aria-label="sign in" className="mx-auto mt-10 flex w-full max-w-sm flex-col gap-4 rounded-card border border-border bg-surface p-6">
      <h2 className="m-0 flex items-center gap-2"><LogIn size={20} aria-hidden className="text-secondary"/>Sign in</h2>
      <form className="flex flex-col" onSubmit={e => { e.preventDefault(); onLogin(username, password); }}>
        <label className="flex flex-col gap-1 text-sm text-secondary">Username <input autoComplete="username" value={username} onChange={e => setUsername(e.target.value)}/></label>
        <label className="flex flex-col gap-1 text-sm text-secondary">Password <input type="password" autoComplete="current-password" value={password} onChange={e => setPassword(e.target.value)}/></label>
        <button type="submit" disabled={busy} className="mt-2 border-text bg-text font-semibold text-on-primary hover:bg-text hover:opacity-85">Sign in</button>
      </form>
      <details className="text-sm text-secondary">
        <summary className="cursor-pointer">Connect with the admin token (setup and maintenance)</summary>
        <form className="flex flex-wrap items-end" onSubmit={e => { e.preventDefault(); onToken(token); }}>
          <label>Admin token <input type="password" value={token} autoComplete="off" onChange={e => setToken(e.target.value)}/></label>
          <button type="submit">Connect</button>
        </form>
      </details>
    </section>
  );
}
