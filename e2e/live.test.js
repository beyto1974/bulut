// Usage: node e2e/live.test.js http://localhost:<port>   (needs playwright and a running app)
// Live updates over the event stream, and the optional bearer token in the settings.
const { chromium } = require('playwright');
const BASE = process.argv[2];

function check(cond, msg) {
  if (!cond) { console.error('FAIL:', msg); process.exitCode = 1; } else { console.log('ok  :', msg); }
}

async function waitFor(fn, ms = 8000) {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    if (await fn()) return true;
    await new Promise((r) => setTimeout(r, 100));
  }
  return false;
}

(async () => {
  const browser = await chromium.launch();
  const ctx = await browser.newContext({ viewport: { width: 1280, height: 900 } });
  const page = await ctx.newPage();
  const problems = [];
  page.on('console', (m) => { if (['error', 'warning'].includes(m.type())) problems.push(`${m.type()}: ${m.text()}`); });
  page.on('pageerror', (e) => problems.push(`pageerror: ${e.message}`));

  const created = await (await fetch(`${BASE}/api/s`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: '{}' })).json();
  const code = created.code;
  await page.goto(`${BASE}/${code}`);
  await page.waitForSelector('#file-count');
  check((await page.locator('#file-count').textContent()).startsWith('0 of'), 'session starts empty');

  // Another client uploads: the open page shows it without a reload.
  await fetch(`${BASE}/api/s/${code}/upload?name=from-elsewhere.txt`, { method: 'PUT', body: 'hello' });
  check(await waitFor(async () => (await page.locator('tbody tr').count()) === 1), 'an upload from elsewhere appears by itself');
  check((await page.locator('tbody tr').first().textContent()).includes('from-elsewhere.txt'), 'with its name');

  // A new version, a note and a rename are picked up too.
  await fetch(`${BASE}/api/s/${code}/upload?name=from-elsewhere.txt`, { method: 'PUT', body: 'hello again' });
  check(await waitFor(async () => (await page.locator('tbody tr').first().textContent()).includes('2 versions')), 'a new version shows as 2 versions');
  await fetch(`${BASE}/api/s`, { method: 'GET' }).catch(() => {});
  await fetch(`${BASE}/api/s/${code}`, { method: 'PATCH', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ description: 'changed elsewhere' }) });
  check(await waitFor(async () => (await page.locator('#description-text').textContent()).includes('changed elsewhere')), 'a new description shows up');

  // Delete from elsewhere.
  const listing = await (await fetch(`${BASE}/api/s/${code}/files`)).json();
  await fetch(`${BASE}/api/s/${code}/nodes/${listing.items[0].id}`, { method: 'DELETE' });
  check(await waitFor(async () => (await page.locator('tbody tr').count()) === 0), 'a delete from elsewhere removes the row');

  // Settings: a token is sent with the page requests, and removing it stops that.
  const seen = [];
  await page.route('**/api/**', (route) => {
    seen.push(route.request().headers()['authorization'] || '');
    route.continue();
  });
  await page.click('#open-settings');
  await page.fill('#token-input', 's3cret');
  await page.click('#settings-save');
  await page.waitForLoadState('load');
  await page.waitForSelector('#file-count');
  check(await waitFor(async () => seen.some((h) => h === 'Bearer s3cret')), 'requests carry the bearer token');
  check(await waitFor(async () => seen.filter((h) => h === 'Bearer s3cret').length >= 2), 'including the event stream');
  check((await page.locator('#open-settings.on').count()) === 1, 'the settings button marks that a token is set');
  await page.click('#open-settings');
  await page.click('#settings-clear');
  await page.waitForLoadState('load');
  await page.waitForSelector('#file-count');
  seen.length = 0;
  await page.reload();
  await page.waitForSelector('#file-count');
  check(!seen.some((h) => h), 'after removing it no request carries a token');

  check(problems.length === 0, 'no console errors or warnings' + (problems.length ? ': ' + problems.join(' | ') : ''));
  await fetch(`${BASE}/api/s/${code}`, { method: 'DELETE' });
  await browser.close();
})();
