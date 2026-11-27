import {createMemoryHistory} from '@tanstack/react-router';
import {describe, expect, it} from 'vitest';
import {pages} from './pages';
import {createStudioRouter} from './router';

describe('routes', () => {
  it('has a route for every page in the navigation', () => {
    const router = createStudioRouter(createMemoryHistory({initialEntries: ['/']}));
    expect(Object.keys(router.routesByPath).sort()).toEqual(pages.map(p => p.to).sort());
  });
  it('matches the addresses the Studio has always used', () => {
    const router = createStudioRouter(createMemoryHistory({initialEntries: ['/']}));
    for (const path of ['/studio', '/audio', '/settings']) expect(router.matchRoutes(path, {}).at(-1)?.routeId).toBe(path);
  });
});
