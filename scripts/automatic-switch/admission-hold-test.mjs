#!/usr/bin/env node
// 開発者用の反例検査。実Claudeのプロセス・入力・認証・設定に触れない。
// 固定したベンダーコードはローカルASARからメモリ内だけで取り出す。
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';

const PIN = Object.freeze({
  version: '2.42304.0',
  sha256: 'bfa22ac393906762fa04b9607ff202907405b089c1ae7db257159202f31bb3d0',
  code: '.vite/build/index.chunk-CFJssAFf.js',
  main: '.vite/build/index.chunk-muI2IV6k.js',
});
const args = process.argv.slice(2);
assert(args.length === 0 || (args.length === 1 && args[0] === '--assert-hold-is-sufficient'),
  'unsupported arguments');

// 検証する条件：受け付け済みの開始処理も終了で中断しない。
// フックまで到達する前の開始が件数から抜けるなら、フックの一時保留を
// 追加しても全体を保護できない。製品の終了拒否を外す検査ではない。
const plist = readFileSync('/Applications/Claude.app/Contents/Info.plist', 'utf8');
assert.equal(plist.match(/<key>CFBundleShortVersionString<\/key>\s*<string>([^<]+)<\/string>/)?.[1],
  PIN.version, 'unreviewed desktop version');
const archive = readFileSync('/Applications/Claude.app/Contents/Resources/app.asar');
assert.equal(createHash('sha256').update(archive).digest('hex'), PIN.sha256,
  'unreviewed desktop archive');
const headerLength = archive.readUInt32LE(12);
assert(headerLength > 0 && headerLength <= 16 * 1024 * 1024, 'invalid ASAR header');
const header = JSON.parse(archive.subarray(16, 16 + headerLength).toString('utf8'));
function chunk(path) {
  let entry = header;
  for (const segment of path.split('/')) entry = entry.files[segment];
  const offset = 8 + archive.readUInt32LE(4) + Number(entry.offset);
  assert(Number.isSafeInteger(offset) && Number.isSafeInteger(entry.size), 'invalid ASAR entry');
  assert(offset >= 16 + headerLength && offset + entry.size <= archive.length,
    'invalid ASAR bounds');
  return archive.subarray(offset, offset + entry.size).toString('utf8');
}
function between(source, start, end) {
  const from = source.indexOf(start);
  assert(from >= 0 && from === source.lastIndexOf(start), 'vendor start is not unique');
  const to = source.indexOf(end, from + start.length);
  assert(to > from && to === source.lastIndexOf(end), 'vendor end is not unique');
  return source.slice(from, to);
}
const code = chunk(PIN.code);
const main = chunk(PIN.main);
// 最初のtoken-cap待機までの実際のprefixだけを実行する。後続の認証、
// Git、worktree、SDK起動は含めない。固定位置と境界を両方検査する。
const startPrefix = code.slice(1719656, 1722344);
assert(startPrefix.startsWith('async startSession(e,r){'), 'unexpected start prefix');
assert(startPrefix.endsWith('let l=await t.Ct();'), 'unexpected first awaited check');
const start = startPrefix.replace('async startSession(', 'async function(')
  + '}catch(e){throw e}}';
const inFlight = between(code, 'var jz=class{', ';async function Mz(').replace('var jz=', '');
const table = between(code, 'GV=class{', ';async function KV(').replace('GV=', '');
const count = between(code, 'countRunningSessions(e){', 'countLiveCliProcesses(e){')
  .replace('countRunningSessions(', 'function(');
const held = between(code, '*heldSessions(){yield*this.sessionTable.heldSessions()}',
  'static{this.PENDING_VISIBILITY_CAP').replace('*heldSessions()', 'function*()');
const aggregate = between(main, 'function Xx(e,t){', 'function rQt(e){');
const install = between(main, 'function lQt(e,t){', 'function uQt(e,t,n){');
const guard = between(main, 'function uQt(e,t,n){', 'async function dQt(e,t,n){');
const capFunctions = [
  between(main, 'function XGa(){', 'function ZGa(){'),
  between(main, 'function ZGa(){', 'async function QGa('),
  between(main, 'async function QGa(', 'function $Ga(){'),
  between(main, 'async function rKa(){', 'async function iKa(){'),
  between(main, 'async function iKa(){', 'function aKa('),
].join('\n');
const cleanup = between(code, 'let s={name:"local-session-stop-all",fn:async()=>{',
  ';this.runQuitStopAll=s.fn');

const context = vm.createContext({}, { codeGeneration: { strings: false, wasm: false } });
// 依存先もVM内の偽物。外部へのI/O、タイマー、process、require、
// 動的importを渡さない。固定コードの必要な分岐以外は例外で止める。
vm.runInContext(`
  var fixture = {
    tokenCapReached: false, afterAwait: false, error: null, warnings: 0,
    cleanupDone: false, appQuitting: false,
  };
  var tokenCap = new Promise(() => {});
  // 実装に存在するtokenLimits設定の経路をモデル化する。利用者の実設定が
  // この経路を使うという主張ではなく、実設定や保存先は読まない。
  var K = () => ({ tokenLimits: { windowHours: 1, maxPerWindow: 100 } });
  var G = () => ({
    orgUuidOverride: () => 'fixture-org', accountIdOverride: () => 'fixture-account',
  });
  var K2t = (account, org) => {
    if (account !== 'fixture-account' || org !== 'fixture-org') throw Error('unexpected selector');
    return '/disposable-fixture/usage.json';
  };
  var TK = { runExclusive: (callback) => callback() };
  var p = { readFile: (path, encoding) => {
    if (path !== '/disposable-fixture/usage.json' || encoding !== 'utf-8') {
      throw Error('unexpected fixture read');
    }
    fixture.tokenCapReached = true;
    return tokenCap;
  } };
  var ua = JSON.parse;
  ${capFunctions}
  var Q = { randomUUID: () => 'fixture-session' };
  var Iz = (cwd) => cwd;
  var t = { XD: () => undefined, Ct: rKa, p5: { info() {} } };
  var q = {}, o = {}, OX = 'fixture stopped';
  class AbortController {
    signal = { aborted: false, addEventListener() {} };
  }
  var sQt = () => false, tS, nS, eS = false, cQt = false;
  var ZOn = (callback) => { fixture.quitGuard = callback; };
  var fQt = () => { throw Error('unexpected quit callback'); };
  var ZZt = () => [];
  var rQt = () => [];
  var dQt = () => { throw Error('unexpected remote guard branch'); };
  var vQt = () => { fixture.warnings += 1; };
  ${aggregate}
  ${install}
  ${guard}
  var InFlight = (${inFlight});
  var Table = (${table});
  var start = (${start});
  var count = (${count});
  var held = (${held});
  var manager = {
    sessions: new Map(), parked: new Map(), spares: new Map(),
    normalizeStartOptionsAtIngress() {},
    startMayGoOnAfterStop: () => true,
    sessionModelSettings: { resolveModel: () => 'fixture-model' },
    assertStartShape() {},
    isInRemoteControlLineage: () => false,
    startingCwds: { reserveStartingCwd() {}, findActiveSessionByCwd: () => undefined },
    restartLosesWorkKind() { throw Error('unexpected session record'); },
    markAppQuitting() { fixture.appQuitting = true; },
    logPendingSessionEndEvents() {}, recordOpenTurnsAtQuit() {},
    markInputPendingWhereExitWriteFailed: () => false,
    resolveQuitStopAllSettled() {},
    persistence: { async flush() {} }, waitingInput: { async flush() {} },
  };
  manager.sessionTable = new Table(manager.sessions, manager.parked, {});
  manager.inFlightStarts = new InFlight({
    held: () => undefined,
    lazyWorktreesAbandon() { throw Error('unexpected abandoned worktree'); },
  });
  manager.heldSessions = held;
  manager.countRunningSessions = count;
  var performance = { now: () => 0 };
  var quitCleanup = (function() { ${cleanup}; return s.fn; }).call(manager);
  lQt([manager], () => false);
  void start.call(manager, { cwd: '/disposable-fixture', message: 'fixture-only' })
    .then(() => { fixture.afterAwait = true; }, (error) => { fixture.error = error.message; });
`, context, { timeout: 1000, filename: 'pinned-admission-prefix' });

// prefixに予期しない失敗があれば、空の一覧を根拠に反例と誤認しない。
for (let i = 0; i < 4; i += 1) await Promise.resolve();
assert.equal(context.fixture.error, null, 'vendor prefix failed before the check');
assert.equal(context.fixture.tokenCapReached, true, 'first awaited check not reached');
assert.equal(context.fixture.afterAwait, false, 'pending check unexpectedly finished');
assert.equal(typeof context.fixture.quitGuard, 'function', 'guard was not installed');
const result = vm.runInContext(`({
  startInFlight: manager.inFlightStarts.inFlightStartCount('local_fixture-session'),
  heldRecords: [...manager.heldSessions()].length,
  countedRunning: manager.countRunningSessions(),
  quitGuardBlocks: fixture.quitGuard(),
})`, context, { timeout: 1000, filename: 'pinned-admission-observation' });
assert.equal(result.startInFlight, 1, 'accepted start is not in flight');
assert.equal(result.heldRecords, 0, 'prefix unexpectedly inserted a session');
assert.equal(result.countedRunning, 0, 'guard count includes start bookkeeping');

if (args[0] === '--assert-hold-is-sufficient') {
  // RED用：安全という仮説は反例があると非0で終了する。
  assert.equal(result.quitGuardBlocks, true,
    'accepted pre-hook start is in flight, but the installed guard permits quit');
} else {
  assert.equal(result.quitGuardBlocks, false);
  assert.equal(context.fixture.warnings, 0);
  process.stdout.write('CASE synthetic token-limit policy; actual cap functions reach fake asynchronous I/O\n');
  process.stdout.write(`COUNTEREXAMPLE ${JSON.stringify(result)}\n`);
  // 実際のCode終了ハンドラーも、一覧登録前の開始は自然完了まで待たない。
  // 他の終了ハンドラーやプロセス終了は実行せず、ここで示す範囲を限定する。
  vm.runInContext(`void quitCleanup().then(
    () => { fixture.cleanupDone = true; },
    (error) => { fixture.error = error.message; },
  );`, context, { timeout: 1000 });
  for (let i = 0; i < 8; i += 1) await Promise.resolve();
  assert.equal(context.fixture.error, null, 'Code quit cleanup failed');
  assert.equal(context.fixture.cleanupDone, true, 'Code quit cleanup did not finish');
  assert.equal(context.fixture.appQuitting, true);
  assert.equal(context.fixture.afterAwait, false);
  assert.equal(vm.runInContext(
    "manager.inFlightStarts.inFlightStartCount('local_fixture-session')", context,
    { timeout: 1000 }), 1);
  process.stdout.write('PASS Code quit cleanup completes while the accepted start remains pending\n');
  // 正の対照：仮の実行中レコードを数えさせれば同じ集計・ガードが拒否する。
  // これは分類器そのものの検証ではなく、常にfalseになる偽物でない確認。
  vm.runInContext(`
    manager.restartLosesWorkKind = () => 'turn';
    manager.sessions.set('tracked', {});
  `, context, { timeout: 1000 });
  assert.equal(vm.runInContext('fixture.quitGuard()', context, { timeout: 1000 }), true);
  assert.equal(context.fixture.warnings, 1);
  process.stdout.write('PASS tracked-record positive control vetoes quit\n');
  process.stdout.write(`PASS pre-hook admission gap reproduced; desktop ${PIN.version}, archive ${PIN.sha256}.\n`);
  process.stdout.write('LIMIT: bounded vendor prefix and fake dependencies, not a real Desktop quit or hook test. Automatic quitting remains disabled.\n');
}
