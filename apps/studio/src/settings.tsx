// Account settings: own password (signed-in users) and user management
// (admins). Roles: admin = everything; operator = run the show (start/stop,
// Program, audio), no scene editing or account management.
import React, {useEffect, useState} from 'react';
import type {createClient, Me, Role, User} from './api/client';

type Api = ReturnType<typeof createClient>;
const ROLE_NAMES: Record<Role, string> = {admin: '管理者', operator: 'オペレーター'};

export function Settings({api, me, onMessage}: {api: Api; me: Me; onMessage: (m: string) => void}) {
  const [users, setUsers] = useState<User[]>([]);
  const [form, setForm] = useState({username: '', password: '', role: 'operator' as Role});
  const [pw, setPw] = useState({current: '', next: ''});
  const admin = me.role === 'admin';

  async function run(f: () => Promise<void>) { try { await f(); } catch (e) { onMessage(String(e)); } }
  const load = async () => { if (admin) setUsers(await api.users()); };
  /** Optimistic: the control changes at once; the server answer (or a reload on error) settles it. */
  function change(u: User, patch: {role?: Role; disabled?: boolean}, done: string) {
    setUsers(xs => xs.map(x => (x.id === u.id ? {...x, ...patch} : x)));
    void run(async () => {
      try { await api.updateUser(u.id, patch); onMessage(done); } finally { await load(); }
    });
  }
  useEffect(() => { void run(load); }, []); // eslint-disable-line react-hooks/exhaustive-deps

  return (
    <div data-testid="settings">
      {me.kind === 'user' && (
        <section aria-label="password">
          <h2>パスワード変更</h2>
          <form onSubmit={e => { e.preventDefault(); void run(async () => { await api.changePassword(pw.current, pw.next); setPw({current: '', next: ''}); onMessage('パスワードを変更しました（他の端末はサインアウトされました）'); }); }}>
            <label>現在のパスワード <input type="password" autoComplete="current-password" value={pw.current} onChange={e => setPw({...pw, current: e.target.value})}/></label>
            <label>新しいパスワード <input type="password" autoComplete="new-password" minLength={10} value={pw.next} onChange={e => setPw({...pw, next: e.target.value})}/></label>
            <button type="submit">変更</button>
          </form>
        </section>
      )}
      {admin && (
        <section aria-label="users">
          <h2>ユーザー</h2>
          <table>
            <thead><tr><th>ユーザー名</th><th>権限</th><th>状態</th><th/></tr></thead>
            <tbody>
              {users.map(u => (
                <tr key={u.id} data-testid={'user-' + u.username}>
                  <td>{u.username}</td>
                  <td><select aria-label={'role ' + u.username} value={u.role} onChange={e => change(u, {role: e.target.value as Role}, '権限を変更しました')}>
                    {(Object.keys(ROLE_NAMES) as Role[]).map(r => <option key={r} value={r}>{ROLE_NAMES[r]}</option>)}</select></td>
                  <td><label>無効 <input type="checkbox" checked={u.disabled} onChange={e => change(u, {disabled: e.target.checked}, e.target.checked ? '無効にしました' : '有効にしました')}/></label></td>
                  <td><button aria-label={'delete ' + u.username} disabled={me.user_id === u.id} onClick={() => void run(async () => { await api.deleteUser(u.id); await load(); onMessage('削除しました'); })}>削除</button></td>
                </tr>
              ))}
            </tbody>
          </table>
          <h3>ユーザー追加</h3>
          <form onSubmit={e => { e.preventDefault(); void run(async () => { await api.createUser(form.username, form.password, form.role); setForm({username: '', password: '', role: 'operator'}); await load(); onMessage('ユーザーを追加しました'); }); }}>
            <label>ユーザー名 <input value={form.username} autoComplete="off" onChange={e => setForm({...form, username: e.target.value})}/></label>
            <label>初期パスワード <input type="password" autoComplete="new-password" value={form.password} onChange={e => setForm({...form, password: e.target.value})}/></label>
            <label>権限 <select value={form.role} onChange={e => setForm({...form, role: e.target.value as Role})}>
              {(Object.keys(ROLE_NAMES) as Role[]).map(r => <option key={r} value={r}>{ROLE_NAMES[r]}</option>)}</select></label>
            <button type="submit">追加</button>
          </form>
        </section>
      )}
    </div>
  );
}
