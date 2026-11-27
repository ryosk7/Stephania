import {expect, test, type Page} from '@playwright/test';
import {Stack, TOKEN} from './stack';

const stack = new Stack();
test.beforeAll(async () => { await stack.start(); });
test.afterAll(async () => { await stack.stop(); });

async function open(page: Page) {
  await page.goto(stack.url);
  await page.getByLabel('管理トークン').fill(TOKEN);
  await expect(page.getByTestId('conn')).toHaveText('ライブ');
  await expect(page.getByTestId('engine')).toHaveText('idle');
}

const status = (page: Page) => page.getByRole('status');

test('live state, multi-tab drafts, conflicts and fail-closed Program', async ({browser}) => {
  const a = await (await browser.newContext()).newPage();
  const b = await (await browser.newContext()).newPage();
  await open(a);
  await open(b);
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
  await b.getByRole('button', {name: 'phone:rear'}).first().click();
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

test('wrong token stops the stream instead of retrying', async ({page}) => {
  await page.goto(stack.url);
  await page.getByLabel('管理トークン').fill('x'.repeat(32));
  await expect(page.getByTestId('conn')).toHaveText('再接続待ち');
});
