// Regenerate the CSW app screenshots in website/assets/ from the real Tauri UI.
//
// The UI (crates/desktop/ui) runs as a "dumb terminal": with no window.__TAURI__
// present it falls back to a dev mock (devInvoke in main.js), so a plain headless
// Chrome renders the real screens with sample data. We drive it over the DevTools
// Protocol and capture at the same recipe as the shipped assets:
//   760 CSS width, deviceScaleFactor 2 (=> 1520px wide), dark theme.
// overview/onboarding/create grow the window to fit their content (the app is a
// fixed viewport with internal scroll + a pinned footer); hero is a 760x498 crop.
//
// Both locales are captured from the SAME app: the UI detects Japanese vs English
// from the browser locale, and a ?lang=ja|en override makes capture deterministic.
// Japanese assets are prefixed ja_ (ja_screen_*.png, ja_hero.png); English assets
// use the bare names (screen_*.png, hero.png). The dev mock returns Japanese sample
// environment names for ja and English ones for en, so each set reads naturally.
//
// Usage: node scripts/appshot/gen-screenshots.mjs
// Requires: Google Chrome, Node 21+ (global WebSocket/fetch).
import http from 'http';
import fs from 'fs';
import path from 'path';
import { spawn } from 'child_process';
import { fileURLToPath } from 'url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const REPO = path.resolve(HERE, '..', '..');
const UI = path.join(REPO, 'crates', 'desktop', 'ui');
const WEBSITE = path.join(REPO, 'website');
const OUT = process.env.CSW_APPSHOT_OUT || path.join(REPO, 'website', 'assets');
fs.mkdirSync(OUT, { recursive: true });
const CHROME = '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const PORT = 8191, DP = 9341;
// The footer version is left out of the screenshots on purpose: without ?appver=
// the dev mock hides it. Screenshots are regenerated in feature PRs, before
// release-please bumps tauri.conf.json, so an injected version always showed the
// previous release on the LP.
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const types = { '.html': 'text/html; charset=utf-8', '.css': 'text/css', '.js': 'text/javascript', '.png': 'image/png', '.svg': 'image/svg+xml' };

const server = http.createServer((req, res) => {
  let u;
  try { u = decodeURIComponent(new URL(req.url, 'http://localhost').pathname); }
  catch { res.statusCode = 400; return res.end('bad path'); }
  const root = u.startsWith('/site/') ? WEBSITE : UI;
  if (root === WEBSITE) u = u.slice('/site'.length);
  if (u.endsWith('/')) u += 'index.html';
  const fp = path.resolve(root, '.' + u);
  if (fp !== root && !fp.startsWith(root + path.sep)) { res.statusCode = 403; return res.end('x'); }
  fs.readFile(fp, (e, d) => { if (e) { res.statusCode = 404; return res.end('nf'); } res.setHeader('Content-Type', types[path.extname(fp)] || 'application/octet-stream'); res.end(d); });
}).listen(PORT, '127.0.0.1');
const chrome = spawn(CHROME, ['--headless=new', '--hide-scrollbars', '--no-first-run', '--no-default-browser-check', `--remote-debugging-port=${DP}`, '--user-data-dir=/tmp/csw-appshot', 'about:blank']);

let id = 0; const pending = new Map(); const errors = [];
async function getWs() { for (let i = 0; i < 60; i++) { try { const r = await fetch(`http://127.0.0.1:${DP}/json/list`); const j = await r.json(); const p = j.find((t) => t.type === 'page' && t.webSocketDebuggerUrl); if (p) return p.webSocketDebuggerUrl; } catch (e) {} await sleep(200); } throw new Error('no devtools'); }

// Per-locale: sample env used for overview/hero, and the sharing-summary matcher.
const LOCALES = [
  { lang: 'ja', prefix: 'ja_', env: '仕事用', summary: /共有|分離/ },
  { lang: 'en', prefix: '', env: 'Work', summary: /shared|isolated/ },
];

async function main() {
  const ws = new WebSocket(await getWs());
  await new Promise((r, j) => { ws.onopen = r; ws.onerror = j; });
  ws.onmessage = (m) => { const d = JSON.parse(m.data); if (d.id && pending.has(d.id)) { pending.get(d.id)(d.result); pending.delete(d.id); } if (d.method === 'Runtime.exceptionThrown') errors.push(d.params.exceptionDetails.text); };
  const cmd = (method, params = {}) => new Promise((res) => { const i = ++id; pending.set(i, res); ws.send(JSON.stringify({ id: i, method, params })); });
  const evalv = (expr) => cmd('Runtime.evaluate', { expression: expr, returnByValue: true, awaitPromise: true });
  await cmd('Page.enable'); await cmd('Runtime.enable');
  await cmd('Emulation.setEmulatedMedia', { features: [{ name: 'prefers-color-scheme', value: 'dark' }] });

  const load = async (lang) => { await cmd('Emulation.setDeviceMetricsOverride', { width: 760, height: 900, deviceScaleFactor: 2, mobile: false }); await cmd('Page.navigate', { url: `http://127.0.0.1:${PORT}/index.html?lang=${lang}` }); await sleep(1300); };
  async function fit(scrollId, file) {
    const over = (await evalv(`(()=>{const s=document.getElementById('${scrollId}');return s?Math.ceil(s.scrollHeight-s.clientHeight):0;})()`)).result?.value || 0;
    await cmd('Emulation.setDeviceMetricsOverride', { width: 760, height: 900 + Math.max(0, over) + 24, deviceScaleFactor: 2, mobile: false });
    await sleep(300);
    const s = await cmd('Page.captureScreenshot', { format: 'png', captureBeyondViewport: false });
    fs.writeFileSync(path.join(OUT, file), Buffer.from(s.data, 'base64'));
  }
  async function crop(file, w, h) {
    await cmd('Emulation.setDeviceMetricsOverride', { width: w, height: h, deviceScaleFactor: 2, mobile: false });
    await sleep(250);
    const s = await cmd('Page.captureScreenshot', { format: 'png', captureBeyondViewport: false });
    fs.writeFileSync(path.join(OUT, file), Buffer.from(s.data, 'base64'));
  }
  const clickEnv = (name) => evalv(`(async()=>{const li=[...document.querySelectorAll('.profile-item')].find(e=>e.querySelector('.profile-name')?.textContent===${JSON.stringify(name)});li&&li.click();await new Promise(r=>setTimeout(r,500));return 'ok';})()`);

  async function checkUsage() {
    // Both locales must remain readable at the app's supported minimum width.
    for (const width of [600, 720]) {
      await cmd('Emulation.setDeviceMetricsOverride', { width, height: 600, deviceScaleFactor: 2, mobile: false });
      await sleep(100);
      const result = (await evalv(`(()=>{
        const text=[...document.querySelectorAll('#viewUsage h1, #viewUsage h2, #viewUsage p, #viewUsage button, .usage-window-heading')];
        return {width:innerWidth, page:document.documentElement.scrollWidth,
          overflow:text.filter(e=>e.scrollWidth>e.clientWidth+1).map(e=>e.className||e.id),
          reports:document.querySelectorAll('.usage-report').length,
          hidden:[...document.querySelectorAll('.view')].filter(e=>e.id!=='viewUsage').every(e=>getComputedStyle(e).display==='none')};
      })()`)).result?.value;
      if (!result || result.page !== result.width || result.overflow.length || result.reports !== 4 || !result.hidden) {
        throw new Error(`usage layout failed: ${JSON.stringify(result)}`);
      }
    }
    const result = (await evalv(`(async()=>{
      const original=usageReports;
      usageReports=[...original, ...Array.from({length:3},(_,i)=>({...original[0],profile:'Synthetic '+i,
        captured_at_ms:Date.now()-31*60000})),
        {...original[0],profile:'<img src=x onerror=alert(1)>'}, {...original[0],profile:'仕事'}];
      renderUsageReports();
      await new Promise(r=>setTimeout(r,0));
      const names=[...document.querySelectorAll('.usage-profile-name')].map(e=>e.textContent);
      const result={count:names.length, literal:names.includes('<img src=x onerror=alert(1)>'),
        untranslated:names.includes('仕事'), injected:!!document.querySelector('#usageReports img'),
        aged:[...document.querySelectorAll('.usage-stale .usage-profile-name')].some(e=>e.textContent==='Synthetic 0'),
        oldValue:[...document.querySelectorAll('.usage-stale .usage-value')].some(e=>e.textContent.includes('78%')),
        unknown:[...document.querySelectorAll('.usage-value')].some(e=>e.textContent==='Unknown'||e.textContent==='不明')};
      usageReports=original;renderUsageReports();return result;
    })()`)).result?.value;
    if (!result || result.count !== 9 || !result.literal || !result.untranslated || result.injected || !result.oldValue || !result.unknown || !result.aged) {
      throw new Error(`usage rendering failed: ${JSON.stringify(result)}`);
    }
  }

  async function checkPendingLaunch(env) {
    const result = (await evalv(`(async()=>{
      await doSwitch(${JSON.stringify(env)});
      const first={...pendingLaunch};
      await doSwitch(${JSON.stringify(env)});
      const duplicate=pendingLaunch.request_id===first.request_id;
      await doSwitch('Other synthetic environment');
      const replacement=pendingLaunch.target===first.target;
      acceptPendingLaunch({...pendingLaunch,error:'process_unavailable'});
      const rechecking=pendingLaunch.phase==='waiting'&&!el.btnCancelPendingLaunch.hidden&&el.pendingLaunchText.textContent===T('Claudeの起動状態を再確認しています。確認できるまで起動せず、予約はキャンセルできます。CSWはClaudeを終了しません。', 'Rechecking Claude’s process state. No launch will begin until it can be verified. You can cancel this reservation. CSW does not quit Claude.');
      await doCancelPendingLaunch();
      const canceled=pendingLaunch.phase==='canceled'&&getComputedStyle(el.pendingLaunchBanner).display==='none';
      await refreshPendingLaunch();
      const stayedCanceled=pendingLaunch.phase==='canceled';
      await doSwitch('<img src=x onerror=alert(1)>');
      const literal=el.pendingLaunchTarget.textContent.includes('<img src=x onerror=alert(1)>')&&!el.pendingLaunchBanner.querySelector('img');
      await doCancelPendingLaunch();
      await doSwitch(${JSON.stringify(env)});
      const waiting={...pendingLaunch};
      window.__DEV_PENDING.switching();
      acceptPendingLaunch(waiting);
      const staleIgnored=pendingLaunch.phase==='switching';
      const cancelHidden=getComputedStyle(el.btnCancelPendingLaunch).display==='none';
      await doCancelPendingLaunch();
      const committed=pendingLaunch.phase==='switching';
      window.__DEV_PENDING.complete();
      const completed=pendingLaunch.phase==='completed'&&getComputedStyle(el.pendingLaunchBanner).display==='none';
      acceptPendingLaunch(waiting);
      const oldIgnored=pendingLaunch.phase==='completed';
      await doSwitch(${JSON.stringify(env)});
      return {duplicate,replacement,rechecking,canceled,stayedCanceled,literal,staleIgnored,cancelHidden,committed,completed,oldIgnored,waiting:pendingLaunch.phase==='waiting'};
    })()`)).result?.value;
    if (!result || Object.values(result).some(v=>v!==true)) throw new Error(`pending launch flow failed: ${JSON.stringify(result)}`);
    for (const error of [null, 'process_unavailable']) {
      await evalv(`acceptPendingLaunch({...pendingLaunch,error:${JSON.stringify(error)}})`);
      for (const width of [600, 720, 760]) {
        await cmd('Emulation.setDeviceMetricsOverride', { width, height: 600, deviceScaleFactor: 2, mobile: false });
        await sleep(100);
        const layout = (await evalv(`(()=>{
          const banner=el.pendingLaunchBanner;
          const nodes=[el.pendingLaunchTarget,el.pendingLaunchText,el.btnCancelPendingLaunch];
          return {width:innerWidth,page:document.documentElement.scrollWidth,visible:getComputedStyle(banner).display!=='none',
            overflow:nodes.filter(e=>e.scrollWidth>e.clientWidth+1).length,
            inside:banner.getBoundingClientRect().right<=innerWidth&&banner.getBoundingClientRect().bottom<=innerHeight,
            buttonLines:el.btnCancelPendingLaunch.scrollHeight<=el.btnCancelPendingLaunch.clientHeight+1};
        })()`)).result?.value;
        if (!layout || layout.width!==layout.page || !layout.visible || layout.overflow || !layout.inside || !layout.buttonLines) {
          throw new Error(`pending launch layout failed: ${JSON.stringify(layout)}`);
        }
      }
    }
    await evalv('acceptPendingLaunch({...pendingLaunch,error:null})');
  }

  async function checkSharedCode(lang) {
    const result = (await evalv(`(async()=>{
      showCreate();
      const defaults=currentMode==='share_code'&&document.querySelector('input[name="mode"]:checked')?.value==='share_code'
        &&overrides.cli_project_memory==='share'&&overrides.cli_history==='share'&&overrides.desktop_worktrees==='copy'
        &&['cli_settings','cli_claude_md','cli_rules','cli_plugins','cli_skills'].every(k=>overrides[k]==='isolate');
      const name=LANG==='ja'?'研究用':'Research';
      await showDetail(name);
      const blocked=document.querySelector('[data-action="setup-code"]')?.disabled===true;
      desktopRunning=false;
      await showDetail(name);
      const calls=DEV_CODE_CALLS.length;
      document.querySelector('[data-action="setup-code"]').click();
      const review=!!document.querySelector('[data-action="confirm-code"]')&&DEV_CODE_CALLS.length===calls;
      document.querySelector('[data-action="cancel-code"]').click();
      await new Promise(r=>setTimeout(r,80));
      const canceled=DEV_CODE_CALLS.length===calls&&!!document.querySelector('[data-action="setup-code"]');
      document.querySelector('[data-action="setup-code"]').click();
      document.querySelector('[data-action="confirm-code"]').click();
      await new Promise(r=>setTimeout(r,80));
      const enabled=DEV_CODE_CALLS.length===calls+1&&DEV_CODE_CALLS.at(-1).name===name
        &&DEV_CODE_CALLS.at(-1).replace===true&&!document.querySelector('[data-action="setup-code"]');
      const repoShared=document.querySelector('.sharing-summary')?.textContent.includes(T('共有3・分離6件','3 shared · 6 isolated'));
      return {defaults,blocked,review,canceled,enabled,repoShared};
    })()`)).result?.value;
    if (!result || Object.values(result).some(v=>v!==true)) throw new Error(`shared Code setup failed (${lang}): ${JSON.stringify(result)}`);
  }

  async function checkLandingPages() {
    async function assertLayout(lang, width, menuOpen) {
      const result = (await evalv(`(()=>{
        const visible=e=>{const s=getComputedStyle(e);return s.visibility!=='hidden'&&s.display!=='none'&&e.getBoundingClientRect().width>0;};
        const metrics=e=>{const s=getComputedStyle(e);return {tag:e.tagName,cls:e.className,text:e.textContent.slice(0,60),
          clientWidth:e.clientWidth,scrollWidth:e.scrollWidth,rectWidth:e.getBoundingClientRect().width,
          visibility:s.visibility,display:s.display,transform:s.transform};};
        const controls=[...document.querySelectorAll('h1,h2,h3,.faq-q,button,.btn,.nav-links a')].filter(visible);
        const nav=document.querySelector('.nav-links');
        const rect=nav?.getBoundingClientRect();
        return {lang:document.documentElement.lang,width:innerWidth,page:document.documentElement.scrollWidth,
          // clientWidth and scrollWidth use the same untransformed layout space.
          overflow:controls.filter(e=>e.clientWidth>0&&e.scrollWidth>e.clientWidth+2).map(metrics),
          menu:{open:document.body.classList.contains('nav-open'),expanded:document.querySelector('.nav-toggle')?.getAttribute('aria-expanded'),
            visible:!!nav&&visible(nav),inside:!!rect&&rect.left>=0&&rect.right<=innerWidth,
            controls:nav?[...nav.querySelectorAll('a,button')].filter(visible).map(metrics):[]}};
      })()`)).result?.value;
      if (!result || result.width !== result.page || result.overflow.length || !result.lang.startsWith(lang)
          || (menuOpen && (!result.menu.open || result.menu.expanded !== 'true' || !result.menu.visible || !result.menu.inside || !result.menu.controls.length))) {
        throw new Error(`landing page layout failed (${lang}, ${width}, menu ${menuOpen ? 'open' : 'closed'}): ${JSON.stringify(result)}`);
      }
    }
    for (const lang of ['en', 'ja']) {
      for (const width of [360, 390, 1024, 1200, 1440]) {
        await cmd('Emulation.setDeviceMetricsOverride', { width, height: 900, deviceScaleFactor: 1, mobile: false });
        await cmd('Page.navigate', { url: `http://127.0.0.1:${PORT}/site/${lang === 'ja' ? 'ja/' : ''}` });
        await sleep(700);
        await assertLayout(lang, width, false);
        if (width <= 768) {
          // Exercise the shipped menu handler rather than forcing a CSS class.
          const clicked = (await evalv(`(()=>{const toggle=document.querySelector('.nav-toggle');
            if(!toggle||getComputedStyle(toggle).display==='none')return false;toggle.click();return true;})()`)).result?.value;
          if (!clicked) throw new Error(`mobile menu toggle unavailable (${lang}, ${width})`);
          // The final child has a 280ms delay plus a 400ms transition.
          await sleep(850);
          await assertLayout(lang, width, true);
        }
      }
    }
  }

  const done = [];
  for (const { lang, prefix, env, summary } of LOCALES) {
    // onboarding: fresh first-run empty view
    await load(lang); await evalv(`localStorage.removeItem('csw_onboarded'); checkFirstRun(); showEmpty();`); await sleep(400);
    await fit('emptyScroll', `${prefix}screen_onboarding.png`);
    // overview: the sample env with the sharing disclosure expanded
    await load(lang); await clickEnv(env);
    await evalv(`(()=>{const s=[...document.querySelectorAll('.sharing-summary')].find(b=>${summary}.test(b.textContent));if(s&&!s.closest('.disclosure').classList.contains('open'))s.click();})()`); await sleep(400);
    await fit('detailScroll', `${prefix}screen_overview.png`);
    // create: the new-environment sheet
    await load(lang); await checkSharedCode(lang);
    await load(lang); await evalv(`showCreate();`); await sleep(400);
    await fit('createScroll', `${prefix}screen_create.png`);
    // usage: all synthetic environments with fresh, stale and missing readings
    await load(lang); await evalv('showUsage()'); await sleep(400);
    await checkUsage();
    await cmd('Emulation.setDeviceMetricsOverride', { width: 760, height: 900, deviceScaleFactor: 2, mobile: false });
    await sleep(150);
    await fit('usageScroll', `${prefix}screen_usage.png`);
    // Manual quit: the real UI with synthetic requests only. No Claude launch,
    // quitting, credentials, OS Accessibility or task state is used by this QA.
    await load(lang); await clickEnv(env);
    await checkPendingLaunch(env);
    // Capture a fresh waiting request, without transient toasts or the mock
    // process state deliberately exercised by the preceding flow checks.
    await load(lang); await clickEnv(env);
    await evalv(`doSwitch(${JSON.stringify(env)})`);
    await crop(`${prefix}screen_pending_launch.png`, 760, 600);
    await evalv("acceptPendingLaunch({...pendingLaunch,error:'process_unavailable'})");
    await crop(`${prefix}screen_pending_recheck.png`, 760, 600);
    // hero: the sample env, disclosures collapsed, 760x498 crop
    await load(lang); await clickEnv(env); await sleep(300);
    await crop(`${prefix}hero.png`, 760, 498);
    done.push(`${prefix}screen_onboarding.png, ${prefix}screen_overview.png, ${prefix}screen_create.png, ${prefix}screen_usage.png, ${prefix}screen_pending_launch.png, ${prefix}screen_pending_recheck.png, ${prefix}hero.png`);
  }

  await checkLandingPages();
  if (errors.length) throw new Error(`uncaught UI exceptions: ${JSON.stringify(errors)}`);
  console.log('regenerated:', done.join(' | '), '; errors:', errors.length ? JSON.stringify(errors) : 'none');
  ws.close();
}
main().then(() => { chrome.kill(); server.close(); process.exit(0); }).catch((e) => { console.error('FAIL', e); chrome.kill(); server.close(); process.exit(1); });
