import {expect, test, type Page} from '@playwright/test';
import {Stack, TOKEN} from './stack';

const stack = new Stack();
test.beforeAll(async () => { await stack.start(); });
test.afterAll(async () => { await stack.stop(); });

async function signInWithToken(page: Page, token = TOKEN) {
  await page.getByText('管理トークンで接続').click();
  await page.getByLabel('管理トークン').fill(token);
  await page.getByRole('button', {name: '接続'}).click();
}

async function open(page: Page, route = '') {
  await page.goto(stack.url + '#/' + route);
  await signInWithToken(page);
  await expect(page.getByTestId('conn')).toHaveText('ライブ');
  await expect(page.getByTestId('engine')).toHaveText('idle');
}

const owner = {headers: {Authorization: 'Bearer ' + TOKEN}};

const status = (page: Page) => page.getByRole('status');

test('live state, multi-tab drafts, conflicts and fail-closed Program', async ({browser}) => {
  const a = await (await browser.newContext()).newPage();
  const b = await (await browser.newContext()).newPage();
  await open(a, 'studio');
  await open(b, 'studio');
  await expect(a.getByTestId('program')).toHaveText('なし');

  // A saves r1; B is told its view is stale and recovers by loading.
  await a.getByRole('button', {name: '下書き保存'}).click();
  await expect(status(a)).toHaveText('Draft saved');
  await expect(a.getByTestId('remote-update')).toHaveCount(0);
  await expect(b.getByTestId('remote-update')).toContainText('r1');
  await b.getByRole('button', {name: '読み込み'}).click();
  await expect(status(b)).toHaveText('Loaded');
  await expect(b.getByTestId('remote-update')).toHaveCount(0);

  // B edits and saves r2; A's stale save is rejected, never silently merged.
  await b.getByRole('region', {name: 'layers'}).getByRole('button', {name: 'phone:rear'}).click();
  await b.getByLabel('x', {exact: true}).fill('1000');
  await b.getByRole('button', {name: '下書き保存'}).click();
  await expect(status(b)).toHaveText('Draft saved');
  await expect(a.getByTestId('remote-update')).toContainText('r2');
  await a.getByRole('button', {name: '下書き保存'}).click();
  await expect(status(a)).toContainText('revision_conflict');

  // Program cannot change while the engine is not streaming.
  await b.getByRole('button', {name: 'Programへ適用'}).click();
  await expect(status(b)).toContainText(/program_inactive|not_implemented/);
  await expect(b.getByTestId('program')).toHaveText('なし');
});

test('engine restart is surfaced live', async ({page}) => {
  await open(page);
  await stack.stopEngine();
  await expect(page.getByTestId('engine')).toHaveText('応答なし');
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
  await expect(panel.getByTestId('audio-state-hdmi')).toHaveText('待機（配信停止中）');
  const mic = panel.getByTestId('audio-mic');
  await mic.getByLabel('デバイス').fill('plughw:CARD=Mic,DEV=0');
  await mic.getByLabel('デバイス').blur();
  await mic.getByLabel('有効').check();
  await expect(page.getByRole('status')).toHaveText('音声設定を適用しました');
  await mic.getByLabel('gain mic').fill('-6');
  await mic.getByRole('button', {name: 'ミュート'}).click();
  await expect(mic.getByRole('button', {name: 'ミュート解除'})).toBeVisible();

  // The engine holds the applied configuration (controls update before the
  // debounced/queued saves land, so wait for the engine side)...
  await expect.poll(async () => {
    const r = await page.request.get(stack.url + 'api/v1/audio', {headers: {Authorization: 'Bearer ' + TOKEN}});
    return (await r.json()).engine.inputs.find((i: {id: string}) => i.id === 'mic');
  }).toMatchObject({enabled: true, device: 'plughw:CARD=Mic,DEV=0', gain_db: -6, muted: true, state: 'idle'});
  // ...and the control plane persisted it across a reload.
  await page.reload();
  await signInWithToken(page);
  await expect(page.getByTestId('audio-mic').getByLabel('有効')).toBeChecked();
  await expect(page.getByTestId('audio-mic').getByLabel('デバイス')).toHaveValue('plughw:CARD=Mic,DEV=0');

  // Invalid device names are refused with a message, nothing applied.
  await page.getByTestId('audio-mic').getByLabel('デバイス').fill('hw:0;reboot');
  await page.getByTestId('audio-mic').getByLabel('デバイス').blur();
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
    await page.getByLabel('ユーザー名').fill(username);
    await page.getByLabel('パスワード').fill('correct horse battery');
    await page.getByRole('button', {name: 'サインイン'}).click();
    await expect(page.getByTestId('conn')).toHaveText('ライブ');
    return page;
  };

  const a = await signIn('alice');
  await expect(a.getByTestId('whoami')).toHaveText('alice（管理者）');
  await a.getByLabel('新しいシーン').fill('Keynote');
  await a.getByRole('button', {name: '作成'}).click();
  await expect(status(a)).toHaveText('シーン「Keynote」を作成しました');
  const layers = a.getByRole('region', {name: 'layers'}).getByRole('listitem');
  await expect(layers).toHaveCount(1);
  await a.getByRole('button', {name: 'レイヤー追加'}).click();
  await expect(layers).toHaveCount(2);
  await a.getByRole('button', {name: '元に戻す'}).click();
  await expect(layers).toHaveCount(1);
  await a.getByRole('button', {name: 'やり直す'}).click();
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
  await a.getByRole('button', {name: '元に戻す'}).click();
  await expect(width).toHaveValue(String(before));
  await a.getByRole('button', {name: 'やり直す'}).click();
  await a.getByRole('button', {name: '下書き保存'}).click();
  await expect(status(a)).toHaveText('Draft saved');
  await expect(a.getByTestId('dirty')).toHaveText('保存済み');

  // The session cookie survives a reload; signing out ends it.
  await a.reload();
  await expect(a.getByTestId('whoami')).toHaveText('alice（管理者）');
  await expect(a.getByRole('region', {name: 'scenes'})).toContainText('Keynote');

  // Operators run the show but cannot edit scenes or manage accounts.
  const o = await signIn('olga');
  await expect(o.getByTestId('whoami')).toHaveText('olga（オペレーター）');
  await expect(o.getByRole('region', {name: 'scenes'})).toContainText('Keynote');
  await expect(o.getByRole('button', {name: '下書き保存'})).toHaveCount(0);
  await expect(o.getByRole('button', {name: 'レイヤー追加'})).toHaveCount(0);
  await o.goto(stack.url + '#/settings');
  await expect(o.getByRole('region', {name: 'users'})).toHaveCount(0);

  // The admin disables the operator: her session ends immediately.
  await a.goto(stack.url + '#/settings');
  await a.getByTestId('user-olga').getByLabel('無効').check();
  await expect(a.getByTestId('user-olga').getByLabel('無効')).toBeChecked();
  const me = await o.request.get(stack.url + 'api/v1/auth/me');
  expect(me.status()).toBe(401);

  await a.getByRole('button', {name: 'サインアウト'}).click();
  await expect(a.getByRole('button', {name: 'サインイン'})).toBeVisible();
  expect((await a.request.get(stack.url + 'api/v1/auth/me')).status()).toBe(401);
});
