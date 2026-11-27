// Signed-in frame: page navigation (icon + word), who is signed in, the
// theme switch, and the live bar; the page renders in the outlet.
import {Link, Outlet} from '@tanstack/react-router';
import {LogOut} from 'lucide-react';
import {LiveBar} from './live-bar';
import {pages} from './pages';
import {useStudio} from './studio-context';
import {ThemeSwitch} from './theme-switch';

export function Shell() {
  const {me, live, conn, logout} = useStudio();
  return (
    <div className="flex flex-col gap-3">
      <nav className="flex flex-wrap items-center gap-1" aria-label="pages">
        {pages.map(({to, label, icon: I}) => (
          <Link key={to} to={to} activeOptions={{exact: true}} activeProps={{'aria-current': 'page'}}
            className="flex items-center gap-2 rounded-button px-3 py-2 text-sm font-medium text-secondary no-underline hover:bg-fill aria-[current=page]:bg-text aria-[current=page]:text-on-primary">
            <I size={16} aria-hidden/>{label}
          </Link>
        ))}
        <span className="flex-1"/>
        <span data-testid="whoami" className="text-sm text-secondary">{me.kind === 'owner' ? 'admin token' : `${me.username} (${me.role})`}</span>
        <ThemeSwitch/>
        <button onClick={() => logout()} aria-label="Sign out" title="Sign out" className="flex h-9 w-9 items-center justify-center rounded-full p-0">
          <LogOut size={16} aria-hidden/>
        </button>
      </nav>
      <LiveBar live={live} conn={conn}/>
      <Outlet/>
    </div>
  );
}
