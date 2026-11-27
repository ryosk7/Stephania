// Pages of the signed-in Studio (TanStack Router). Hash history keeps the
// existing addresses (#/studio, #/audio, #/settings) and needs nothing from
// the server, which serves one index.html.
import {Navigate, createHashHistory, createRootRoute, createRoute, createRouter, type RouterHistory} from '@tanstack/react-router';
import {AudioPanel} from './audio';
import {Dashboard} from './dashboard';
import {Settings} from './settings';
import {Shell} from './shell';
import {Studio} from './studio';
import {useStudio} from './studio-context';

const rootRoute = createRootRoute({component: Shell, notFoundComponent: () => <Navigate to="/" replace />});

const dashboard = createRoute({getParentRoute: () => rootRoute, path: '/', component: Dashboard});
const studio = createRoute({getParentRoute: () => rootRoute, path: '/studio', component: function StudioPage() {
  const {api, live, canEdit, onMessage} = useStudio();
  return <Studio api={api} live={live} canEdit={canEdit} onMessage={onMessage}/>;
}});
const audio = createRoute({getParentRoute: () => rootRoute, path: '/audio', component: function AudioPage() {
  const {api, live, onMessage} = useStudio();
  return <AudioPanel api={api} levels={live.audio} onMessage={onMessage}/>;
}});
const settings = createRoute({getParentRoute: () => rootRoute, path: '/settings', component: function SettingsPage() {
  const {api, me, onMessage} = useStudio();
  return <Settings api={api} me={me} onMessage={onMessage}/>;
}});

export const routeTree = rootRoute.addChildren([dashboard, studio, audio, settings]);

export function createStudioRouter(history: RouterHistory = createHashHistory()) {
  return createRouter({routeTree, history});
}

declare module '@tanstack/react-router' {
  interface Register {router: ReturnType<typeof createStudioRouter>}
}
