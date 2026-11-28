import {expect, test, type Page} from '@playwright/test';
import {Stack, TOKEN} from './stack';

const stack = new Stack();
test.beforeAll(async () => { await stack.start(); });
test.afterAll(async () => { await stack.stop(); });

async function signInWithToken(page: Page, token = TOKEN) {
  await page.getByText('Connect with the admin token').click();
  await page.getByLabel('Admin token').fill(token);
  await page.getByRole('button', {name: 'Connect'}).click();
}

async function open(page: Page, route = '') {
  await page.goto(stack.url + '#/' + route);
  await signInWithToken(page);
  await expect(page.getByTestId('conn')).toHaveText('live');
  await expect(page.getByTestId('engine')).toHaveText('idle');
}

const owner = {headers: {Authorization: 'Bearer ' + TOKEN}};

const status = (page: Page) => page.getByRole('status');

test('live state, multi-tab drafts, conflicts and fail-closed Program', async ({browser}) => {
  const a = await (await browser.newContext()).newPage();
  const b = await (await browser.newContext()).newPage();
  await open(a, 'studio');
  await open(b, 'studio');
  await expect(a.getByTestId('program')).toHaveText('none');

  // A saves r1; B is told its view is stale and recovers by loading.
  await a.getByRole('button', {name: 'Save draft'}).click();
  await expect(status(a)).toHaveText('Draft saved');
  await expect(a.getByTestId('remote-update')).toHaveCount(0);
  await expect(b.getByTestId('remote-update')).toContainText('r1');
  await b.getByRole('button', {name: 'Reload'}).click();
  await expect(status(b)).toHaveText('Loaded');
  await expect(b.getByTestId('remote-update')).toHaveCount(0);

  // B edits and saves r2; A's stale save is rejected, never silently merged.
  await b.getByRole('region', {name: 'layers'}).getByRole('button', {name: 'phone:rear'}).click();
  await b.getByLabel('x', {exact: true}).fill('1000');
  await b.getByRole('button', {name: 'Save draft'}).click();
  await expect(status(b)).toHaveText('Draft saved');
  await expect(a.getByTestId('remote-update')).toContainText('r2');
  await a.getByRole('button', {name: 'Save draft'}).click();
  await expect(status(a)).toContainText('revision_conflict');

  // Program cannot change while the engine is not streaming.
  await b.getByRole('button', {name: 'Apply to Program'}).click();
  await expect(status(b)).toContainText(/program_inactive|not_implemented/);
  await expect(b.getByTestId('program')).toHaveText('none');
});

test('engine restart is surfaced live', async ({page}) => {
  await open(page);
  await stack.stopEngine();
  await expect(page.getByTestId('engine')).toHaveText('not responding');
  await stack.startEngine();
  await expect(page.getByTestId('restarted')).toBeVisible();
  await expect(page.getByTestId('engine')).toHaveText('idle');
});

test('wrong token is refused and nothing opens', async ({page}) => {
  await page.goto(stack.url);
  await signInWithToken(page, 'x'.repeat(32));
  await expect(status(page)).toContainText('unauthorized');
  await expect(page.getByTestId('live')).toHaveCount(0);
});

test('audio inputs are configured, persisted and applied to the engine', async ({page}) => {
  await open(page, 'audio');
  const panel = page.getByTestId('audio');
  await expect(panel.getByTestId('audio-state-hdmi')).toHaveText('idle (not streaming)');
  const mic = panel.getByTestId('audio-mic');
  await mic.getByLabel('Device').fill('plughw:CARD=Mic,DEV=0');
  await mic.getByLabel('Device').blur();
  await mic.getByLabel('Enabled').check();
  await expect(page.getByRole('status')).toHaveText('Audio settings applied');
  await mic.getByLabel('gain mic').fill('-6');
  await mic.getByRole('button', {name: 'Mute', exact: true}).click();
  await expect(mic.getByRole('button', {name: 'Unmute'})).toBeVisible();

  // The engine holds the applied configuration (controls update before the
  // debounced/queued saves land, so wait for the engine side)...
  await expect.poll(async () => {
    const r = await page.request.get(stack.url + 'api/v1/audio', {headers: {Authorization: 'Bearer ' + TOKEN}});
    return (await r.json()).engine.inputs.find((i: {id: string}) => i.id === 'mic');
  }).toMatchObject({enabled: true, device: 'plughw:CARD=Mic,DEV=0', gain_db: -6, muted: true, state: 'idle'});
  // ...and the control plane persisted it across a reload.
  await page.reload();
  await signInWithToken(page);
  await expect(page.getByTestId('audio-mic').getByLabel('Enabled')).toBeChecked();
  await expect(page.getByTestId('audio-mic').getByLabel('Device')).toHaveValue('plughw:CARD=Mic,DEV=0');

  // Invalid device names are refused with a message, nothing applied.
  await page.getByTestId('audio-mic').getByLabel('Device').fill('hw:0;reboot');
  await page.getByTestId('audio-mic').getByLabel('Device').blur();
  await expect(page.getByRole('status')).toContainText('invalid_audio');
});

test('accounts: admin edits scenes with undo/redo and resize, operator cannot, sessions end', async ({browser}) => {
  const req = await (await browser.newContext()).request;
  for (const [username, role] of [['alice', 'admin'], ['olga', 'operator']]) {
    const r = await req.post(stack.url + 'api/v1/users', {...owner, data: {username, password: 'correct horse battery', role}});
    expect(r.status()).toBe(201);
  }
  const signIn = async (username: string) => {
    const page = await (await browser.newContext()).newPage();
    await page.goto(stack.url + '#/studio');
    await page.getByLabel('Username').fill(username);
    await page.getByLabel('Password').fill('correct horse battery');
    await page.getByRole('button', {name: 'Sign in'}).click();
    await expect(page.getByTestId('conn')).toHaveText('live');
    return page;
  };

  const a = await signIn('alice');
  await expect(a.getByTestId('whoami')).toHaveText('alice (admin)');
  await a.getByLabel('New scene').fill('Keynote');
  await a.getByRole('button', {name: 'Create'}).click();
  await expect(status(a)).toHaveText('Created scene "Keynote"');
  const layers = a.getByRole('region', {name: 'layers'}).getByRole('listitem');
  await expect(layers).toHaveCount(1);
  await a.getByRole('button', {name: 'Add layer', exact: true}).click();
  await expect(layers).toHaveCount(2);
  await a.getByRole('button', {name: 'Undo'}).click();
  await expect(layers).toHaveCount(1);
  await a.getByRole('button', {name: 'Redo'}).click();
  await expect(layers).toHaveCount(2);

  // Resize the new layer by dragging its south-east handle.
  const width = a.getByLabel('width', {exact: true});
  const before = Number(await width.inputValue());
  const handle = a.locator('[data-handle=se]');
  const box = (await handle.boundingBox())!;
  await a.mouse.move(box.x + 3, box.y + 3);
  await a.mouse.down();
  await a.mouse.move(box.x - 60, box.y + 40, {steps: 5});
  await a.mouse.up();
  await expect.poll(async () => Number(await width.inputValue())).toBeLessThan(before);
  // The whole drag is one undo step.
  await a.getByRole('button', {name: 'Undo'}).click();
  await expect(width).toHaveValue(String(before));
  await a.getByRole('button', {name: 'Redo'}).click();
  await a.getByRole('button', {name: 'Save draft'}).click();
  await expect(status(a)).toHaveText('Draft saved');
  await expect(a.getByTestId('dirty')).toHaveText('saved');

  // The session cookie survives a reload; signing out ends it.
  await a.reload();
  await expect(a.getByTestId('whoami')).toHaveText('alice (admin)');
  await expect(a.getByRole('region', {name: 'scenes'})).toContainText('Keynote');

  // Operators run the show but cannot edit scenes or manage accounts.
  const o = await signIn('olga');
  await expect(o.getByTestId('whoami')).toHaveText('olga (operator)');
  await expect(o.getByRole('region', {name: 'scenes'})).toContainText('Keynote');
  await expect(o.getByRole('button', {name: 'Save draft'})).toHaveCount(0);
  await expect(o.getByRole('button', {name: 'Add layer', exact: true})).toHaveCount(0);
  await o.goto(stack.url + '#/settings');
  await expect(o.getByRole('region', {name: 'users'})).toHaveCount(0);
  await expect(o.getByTestId('output-status')).toHaveText(/^Not configured/);

  // The admin sets the stream destination; the key is never shown again,
  // and the operator only learns that one is configured.
  await a.goto(stack.url + '#/settings');
  const key = 'e2e0-test-key0-wxyz';
  await a.getByLabel('Stream key').fill(key);
  await a.getByRole('button', {name: 'Save', exact: true}).click();
  await expect(status(a)).toHaveText(/^Stream destination saved/);
  await expect(a.getByTestId('output-status')).toHaveText('Configured: rtmps://a.rtmps.youtube.com/live2 · key ending wxyz');
  await expect(a.getByLabel('Stream key')).toHaveValue('');
  expect(await (await a.request.get(stack.url + 'api/v1/output')).text()).not.toContain(key);
  await o.reload();
  await expect(o.getByTestId('output-status')).toHaveText('Configured');
  await expect(o.getByLabel('Stream key')).toHaveCount(0);
  await a.getByRole('button', {name: 'Remove'}).click();
  await expect(a.getByTestId('output-status')).toHaveText(/^Not configured/);

  // The admin disables the operator: her session ends immediately.
  await a.getByTestId('user-olga').getByLabel('Disabled').check();
  await expect(status(a)).toHaveText('Disabled');
  const me = await o.request.get(stack.url + 'api/v1/auth/me');
  expect(me.status()).toBe(401);

  await a.getByRole('button', {name: 'Sign out'}).click();
  await expect(a.getByRole('button', {name: 'Sign in'})).toBeVisible();
  expect((await a.request.get(stack.url + 'api/v1/auth/me')).status()).toBe(401);
});

// 2×2 opaque red PNG.
const RED_PNG = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAEUlEQVR4nGP4z8DwH4QZYAwAR8oH+WdZbrcAAAAASUVORK5CYII=', 'base64');

test('scene v2: convert, colour layer, opacity, PNG asset, rendered template, fade', async ({page}) => {
  // A legacy v1 draft is converted explicitly.
  const put = await page.request.put(stack.url + 'api/v1/scenes/brand', {...owner, data: {revision: 0, document:
    {version: 1, name: 'Brand', layers: [{id: 'hdmi', source: 'hdmi', x: 0, y: 0, width: 1920, height: 1080, visible: true}]}}});
  expect(put.status()).toBe(200);
  await open(page, 'studio');
  await page.getByRole('region', {name: 'scenes'}).getByRole('button', {name: /Brand/}).click();
  await expect(status(page)).toHaveText('Loaded');
  await expect(page.getByTestId('v1-banner')).toBeVisible();
  await expect(page.getByRole('button', {name: 'Add colour layer'})).toBeDisabled();
  await page.getByRole('button', {name: 'Convert to v2'}).click();
  await expect(page.getByTestId('v1-banner')).toHaveCount(0);

  await page.getByRole('button', {name: 'Add colour layer'}).click();
  await page.getByLabel('opacity', {exact: true}).fill('40');
  await page.getByLabel('rotation', {exact: true}).fill('5');

  await page.getByLabel('Upload PNG').setInputFiles({name: 'red.png', mimeType: 'image/png', buffer: RED_PNG});
  await expect(status(page)).toContainText('Uploaded');
  await page.getByRole('region', {name: 'assets'}).getByRole('button', {name: 'Add to scene'}).click();
  await expect(page.locator('.layer.image img')).toHaveCount(1);

  await page.getByRole('region', {name: 'templates'}).getByLabel('Speaker name').fill('Jane <Doe>');
  await page.getByRole('button', {name: 'Add template'}).click();
  await expect(status(page)).toHaveText('Added "Lower third"');
  await expect(page.getByLabel('var speaker.name')).toHaveValue('Jane <Doe>');

  await page.getByRole('button', {name: 'Save draft'}).click();
  await expect(status(page)).toHaveText('Draft saved');
  const scenes = await (await page.request.get(stack.url + 'api/v1/scenes', owner)).json();
  const doc = scenes.find((s: {id: string}) => s.id === 'brand').document;
  expect(doc.version).toBe(2);
  const types = doc.layers.map((l: {type: string}) => l.type);
  expect(types).toEqual(['video', 'color', 'image', 'image']);
  expect(doc.layers[1]).toMatchObject({opacity: 40, rotation: 5});
  expect(doc.layers[3].template).toEqual({id: 'lower-third', vars: {'speaker.name': 'Jane <Doe>', 'speaker.title': 'Affiliation and title', 'event.name': 'Event name'}});
  // The rendered template is a real PNG of the template's size.
  const asset = (await (await page.request.get(stack.url + 'api/v1/assets', owner)).json()).find((a: {id: string}) => a.id === doc.layers[3].asset);
  expect(asset).toMatchObject({width: 1000, height: 160});

  // Re-rendering after a variable change stores a new asset on the layer.
  await page.getByLabel('var speaker.name').fill('Another Speaker');
  await page.getByRole('button', {name: 'Re-render'}).click();
  await expect(status(page)).toContainText('re-rendered');
  await page.getByRole('button', {name: 'Save draft'}).click();
  await expect(status(page)).toHaveText('Draft saved');
  const after = (await (await page.request.get(stack.url + 'api/v1/scenes/brand', owner)).json()).document;
  expect(after.layers[3].asset).not.toBe(doc.layers[3].asset);
  expect(after.layers[3].template.vars['speaker.name']).toBe('Another Speaker');

  // The asset in use cannot be deleted; fade is offered and fails closed without a stream.
  const del = await page.request.delete(stack.url + 'api/v1/assets/' + after.layers[2].asset, owner);
  expect(del.status()).toBe(409);
  await page.getByLabel('transition').selectOption('fade');
  await page.getByLabel('fade ms').fill('800');
  await page.getByRole('button', {name: 'Apply to Program'}).click();
  await expect(status(page)).toContainText('program_inactive');
});
