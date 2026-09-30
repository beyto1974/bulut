// Usage: node e2e/ui.test.js http://localhost:<port>   (needs playwright and a running app)
// Drives the real UI in Chromium: create a session, upload two versions, tag, note, QR, themes, phone width.
const { chromium } = require('playwright');
const BASE = process.argv[2];
const OUT = process.env.SHOTS_DIR || require('os').tmpdir() + '/bulut-shots';
require('fs').mkdirSync(OUT, { recursive: true });

function check(cond, msg) {
  if (!cond) { console.error('FAIL:', msg); process.exitCode = 1; } else { console.log('ok  :', msg); }
}

(async () => {
  const browser = await chromium.launch();
  const ctx = await browser.newContext({ viewport: { width: 1280, height: 900 }, colorScheme: 'light' });
  const page = await ctx.newPage();
  const problems = [];
  page.on('console', (m) => { if (['error', 'warning'].includes(m.type())) problems.push(`${m.type()}: ${m.text()}`); });
  page.on('pageerror', (e) => problems.push(`pageerror: ${e.message}`));

  // Home
  await page.goto(BASE + '/');
  await page.waitForSelector('#new-session');
  check((await page.title()).startsWith('Bulut'), 'home title ' + (await page.title()));
  check(await page.locator('#env-badge').isVisible(), 'environment badge is visible outside production');
  check((await page.locator('#footer-version').textContent()).includes('bulut v'), 'version in footer');
  const info = await page.locator('#agent-info').textContent();
  check(/REST API/.test(info) && /llms\.txt/.test(info) && /MCP/.test(info), 'home page says it is agent friendly (REST, llms.txt, MCP)');
  check((await page.locator('#agent-info a[href="/openapi.json"]').count()) === 1, 'home page links to openapi.json');
  await page.screenshot({ path: `${OUT}/home.png` });

  // New session
  await page.click('#new-session');
  await page.waitForSelector('#session-code');
  const code = await page.locator('#session-code').textContent();
  check(/^[a-hj-km-np-z2-9]{5}$/.test(code), 'code has 5 characters without look-alikes: ' + code);
  check(page.url().endsWith('/' + code), 'url is the short link');
  check((await page.title()).startsWith(code), 'title names the code: ' + (await page.title()));

  // Description
  await page.click('#edit-description');
  await page.fill('#description-input', 'Regression files for the invoice parser');
  await page.click('#save-description');
  await page.waitForSelector('#description-text');
  check((await page.locator('#description-text').textContent()).includes('invoice parser'), 'description saved');
  check((await page.locator('#expiry').textContent()).includes('7 day timer'), 'expiry text mentions the 7 day timer');

  // Upload two versions of build.apk and a text file
  const apk1 = Buffer.alloc(3 * 1024 * 1024, 1);
  // 17 MiB plus a bit: three parts of 8 MiB, so the chunked path is exercised for real.
  const apk2 = Buffer.alloc(17 * 1024 * 1024 + 123, 2);
  await page.setInputFiles('#file-input', { name: 'build.apk', mimeType: 'application/vnd.android.package-archive', buffer: apk1 });
  await page.waitForSelector('tr.row-item');
  await page.waitForFunction(() => document.querySelector('#queue .q-status')?.textContent.startsWith('Done'));
  await page.setInputFiles('#file-input', { name: 'build.apk', mimeType: 'application/vnd.android.package-archive', buffer: apk2 });
  await page.waitForFunction(() => [...document.querySelectorAll('#queue .q-status')].filter((s) => s.textContent.startsWith('Done')).length === 2);
  await page.setInputFiles('#file-input', { name: 'notes.txt', mimeType: 'text/plain', buffer: Buffer.from('hello from the test') });
  await page.waitForFunction(() => document.querySelectorAll('tr.row-item').length === 2);
  check((await page.locator('tr.row-item').count()) === 2, 'two files listed (second upload of build.apk is a version)');
  const apkRow = page.locator('tr.row-item', { hasText: 'build.apk' });
  check((await apkRow.textContent()).includes('2 versions'), 'row shows 2 versions');

  // Several files chosen at once upload in parallel and all arrive.
  await page.setInputFiles('#file-input', [
    { name: 'p1.txt', mimeType: 'text/plain', buffer: Buffer.from('one') },
    { name: 'p2.txt', mimeType: 'text/plain', buffer: Buffer.from('two') },
    { name: 'p3.txt', mimeType: 'text/plain', buffer: Buffer.from('three') },
    { name: 'p4.txt', mimeType: 'text/plain', buffer: Buffer.from('four') },
  ]);
  await page.waitForFunction(() => document.querySelectorAll('tr.row-item').length === 6);
  await page.waitForFunction(() => [...document.querySelectorAll('#queue .q-status')].every((s) => s.textContent.startsWith('Done')));
  check(true, 'four files chosen together all uploaded');
  for (const n of ['p1', 'p2', 'p3', 'p4']) {
    await page.locator('tr.row-item', { hasText: n + '.txt' }).click();
    await page.click('#delete-file');
    await page.click('#delete-file');
    await page.waitForFunction((name) => ![...document.querySelectorAll('tr.row-item')].some((r) => r.textContent.includes(name)), n + '.txt');
  }

  // Inspector: versions, tags, note
  await apkRow.click();
  await page.waitForSelector('#versions');
  check((await page.locator('#versions .ver').count()) === 2, 'two versions in the inspector');
  check((await page.locator('#tags').textContent()).includes('latest'), 'newest version carries latest');
  await page.fill('#tag-input', 'v1.4.2');
  await page.press('#tag-input', 'Enter');
  await page.waitForFunction(() => document.querySelector('#tags')?.textContent.includes('v1.4.2'));
  check(true, 'tag added');
  await page.locator('#versions .ver').nth(1).click();
  check(!(await page.locator('#tags').textContent()).includes('latest'), 'older version has no latest tag');
  check((await page.locator('#meta-created').textContent()) !== 'unknown', 'created time is shown (file timestamp)');
  check(/\d{4}-\d{2}-\d{2} \d{2}:\d{2}/.test(await page.locator('#meta-uploaded').textContent()), 'uploaded time is shown');
  await page.fill('#note-input', 'Crashes on invoice-0412');
  await page.click('#save-note');
  await page.waitForSelector('.dot');
  check(true, 'note saved, dot shown in the table');
  const href = await page.locator('#download').getAttribute('href');
  const dl = await page.request.get(BASE + href);
  check(dl.status() === 200 && (await dl.body()).length === apk1.length, 'download of the older version returns its bytes');

  // QR dialog
  await page.click('#show-qr');
  await page.waitForSelector('#qr-dialog[open]');
  await page.waitForFunction(() => document.querySelector('#qr-image').complete && document.querySelector('#qr-image').naturalWidth > 0);
  check((await page.locator('#qr-url').textContent()) === `${BASE}/${code}`, 'QR dialog shows the link ' + BASE + '/' + code);
  await page.screenshot({ path: `${OUT}/qr.png` });
  await page.click('#qr-close');

  // Screenshots: light, dark, phone
  await page.locator('tr.row-item', { hasText: 'build.apk' }).click();
  await page.screenshot({ path: `${OUT}/session-light.png`, fullPage: true });
  await page.emulateMedia({ colorScheme: 'dark' });
  await page.screenshot({ path: `${OUT}/session-dark.png`, fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({ path: `${OUT}/session-phone.png`, fullPage: true });
  const overflow = await page.evaluate(() => document.documentElement.scrollWidth > document.documentElement.clientWidth + 1);
  check(!overflow, 'no horizontal page scroll at phone width');

  // Delete a file
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.locator('tr.row-item', { hasText: 'notes.txt' }).click();
  await page.click('#delete-file');
  await page.click('#delete-file');
  await page.waitForFunction(() => document.querySelectorAll('tr.row-item').length === 1);
  check(true, 'file deleted after two clicks');

  // Everything so far must have been free of console errors and CSP violations.
  check(problems.length === 0, 'no console errors or CSP violations' + (problems.length ? ': ' + problems.join(' | ') : ''));
  problems.length = 0;

  // Unknown session and a non-code path (the browser logs the expected 404 for the API call)
  await page.goto(BASE + '/k7m3q');
  await page.waitForSelector('h1');
  check((await page.locator('h1').textContent()) === 'Session not found', 'unknown session shows a not-found page');
  const fav = await page.request.get(BASE + '/favicon.ico');
  check(fav.status() === 404, 'favicon path is a 404');

  await page.request.delete(`${BASE}/api/s/${code}`);
  await browser.close();
})().catch((e) => { console.error('ERROR', e); process.exit(2); });
