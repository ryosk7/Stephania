// Settings: the stream destination (admins set it; everyone sees whether
// one is configured), own password (signed-in users) and user management
// (admins). Roles: admin = everything; operator = run the show (start/stop,
// Program, audio), no scene editing or account management.
import React, {useEffect, useState} from 'react';
import type {createClient, Me, OutputView, Role, User} from './api/client';
import {outputProblem, outputSummary, YOUTUBE_URL} from './output';

type Api = ReturnType<typeof createClient>;
const ROLE_NAMES: Record<Role, string> = {admin: 'Admin', operator: 'Operator'};

export function Settings({api, me, onMessage}: {api: Api; me: Me; onMessage: (m: string) => void}) {
  const [users, setUsers] = useState<User[]>([]);
  const [form, setForm] = useState({username: '', password: '', role: 'operator' as Role});
  const [pw, setPw] = useState({current: '', next: ''});
  const admin = me.role === 'admin';
  const [output, setOutput] = useState<OutputView>();
  const [dest, setDest] = useState({url: YOUTUBE_URL, key: ''});

  async function run(f: () => Promise<void>) { try { await f(); } catch (e) { onMessage(String(e)); } }
  const load = async () => { if (admin) setUsers(await api.users()); };
  /** Optimistic: the control changes at once; the server answer (or a reload on error) settles it. */
  function change(u: User, patch: {role?: Role; disabled?: boolean}, done: string) {
    setUsers(xs => xs.map(x => (x.id === u.id ? {...x, ...patch} : x)));
    void run(async () => {
      try { await api.updateUser(u.id, patch); onMessage(done); } finally { await load(); }
    });
  }
  const loadOutput = async () => { setOutput(await api.output()); };
  useEffect(() => { void run(load); void run(loadOutput); }, []); // eslint-disable-line react-hooks/exhaustive-deps
  function saveOutput() {
    const problem = outputProblem(dest.url, dest.key);
    if (problem) { onMessage(problem); return; }
    void run(async () => {
      // The key is kept only in this form until it is sent; the answer has its last 4 characters.
      setOutput(await api.saveOutput(dest.url.trim(), dest.key.trim()));
      setDest({...dest, key: ''});
      onMessage('Stream destination saved; it applies at the next stream start');
    });
  }

  return (
    <div data-testid="settings">
      <section aria-label="stream destination">
        <h2>Stream destination</h2>
        <p data-testid="output-status">{outputSummary(output)}</p>
        {admin && (
          <form onSubmit={e => { e.preventDefault(); saveOutput(); }}>
            <label>Server URL <input value={dest.url} autoComplete="off" spellCheck={false} onChange={e => setDest({...dest, url: e.target.value})}/></label>
            <label>Stream key <input type="password" value={dest.key} autoComplete="off" spellCheck={false}
              placeholder={output?.configured ? 'Enter a new key to replace it' : 'From YouTube Studio → Go live → Stream'} onChange={e => setDest({...dest, key: e.target.value})}/></label>
            <button type="submit">{output?.configured ? 'Replace' : 'Save'}</button>
            {output?.configured && (
              <button type="button" onClick={() => void run(async () => { await api.clearOutput(); await loadOutput(); onMessage('Stream destination removed'); })}>Remove</button>
            )}
          </form>
        )}
      </section>
      {me.kind === 'user' && (
        <section aria-label="password">
          <h2>Change password</h2>
          <form onSubmit={e => { e.preventDefault(); void run(async () => { await api.changePassword(pw.current, pw.next); setPw({current: '', next: ''}); onMessage('Password changed (other devices were signed out)'); }); }}>
            <label>Current password <input type="password" autoComplete="current-password" value={pw.current} onChange={e => setPw({...pw, current: e.target.value})}/></label>
            <label>New password <input type="password" autoComplete="new-password" minLength={10} value={pw.next} onChange={e => setPw({...pw, next: e.target.value})}/></label>
            <button type="submit">Change</button>
          </form>
        </section>
      )}
      {admin && (
        <section aria-label="users">
          <h2>Users</h2>
          <table>
            <thead><tr><th>Username</th><th>Role</th><th>State</th><th/></tr></thead>
            <tbody>
              {users.map(u => (
                <tr key={u.id} data-testid={'user-' + u.username}>
                  <td>{u.username}</td>
                  <td><select aria-label={'role ' + u.username} value={u.role} onChange={e => change(u, {role: e.target.value as Role}, 'Role changed')}>
                    {(Object.keys(ROLE_NAMES) as Role[]).map(r => <option key={r} value={r}>{ROLE_NAMES[r]}</option>)}</select></td>
                  <td><label>Disabled <input type="checkbox" checked={u.disabled} onChange={e => change(u, {disabled: e.target.checked}, e.target.checked ? 'Disabled' : 'Enabled')}/></label></td>
                  <td><button aria-label={'delete ' + u.username} disabled={me.user_id === u.id} onClick={() => void run(async () => { await api.deleteUser(u.id); await load(); onMessage('Deleted'); })}>Delete</button></td>
                </tr>
              ))}
            </tbody>
          </table>
          <h3>Add user</h3>
          <form onSubmit={e => { e.preventDefault(); void run(async () => { await api.createUser(form.username, form.password, form.role); setForm({username: '', password: '', role: 'operator'}); await load(); onMessage('User added'); }); }}>
            <label>Username <input value={form.username} autoComplete="off" onChange={e => setForm({...form, username: e.target.value})}/></label>
            <label>Initial password <input type="password" autoComplete="new-password" value={form.password} onChange={e => setForm({...form, password: e.target.value})}/></label>
            <label>Role <select value={form.role} onChange={e => setForm({...form, role: e.target.value as Role})}>
              {(Object.keys(ROLE_NAMES) as Role[]).map(r => <option key={r} value={r}>{ROLE_NAMES[r]}</option>)}</select></label>
            <button type="submit">Add</button>
          </form>
        </section>
      )}
    </div>
  );
}
