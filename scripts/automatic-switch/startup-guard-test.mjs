#!/usr/bin/env node
// 開発者用の隔離テスト。製品の起動・終了許可には使用しない。
// ベンダーコードはハッシュ固定のローカル ASAR からメモリ内だけで抽出する。
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { EventEmitter } from 'node:events';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';

const PIN = Object.freeze({
  version: '2.42304.0',
  sha256: 'bfa22ac393906762fa04b9607ff202907405b089c1ae7db257159202f31bb3d0',
  chunk: '.vite/build/index.chunk-muI2IV6k.js',
});
const FUNCTIONS = [
  'So', 'Yx', 'Xx', 'rQt', 'aQt', '$x', 'oQt',
  'lQt', 'uQt', 'fQt', '_Qt', 'vQt',
];
const START = 'So(6e4).catch((()=>void 0)).then((async()=>{let{claudeCodeSessionManager:e}';
const INSTALLED = 'n([e,t],AR)';
const ARGS = process.argv.slice(2);
assert(ARGS.length === 0 || (ARGS.length === 1 && ARGS[0] === '--assert-five-second-readiness'),
  'unsupported arguments');

// テスト仕様（実装から逆算しない期待値）:
// 1. 経過時間・レンダラーの loaded だけを guard 登録の証拠にしない。
// 2. 起動待ちのタイムアウトも登録より前にあることを再現する。
// 3. Wait は turn / held input / background work が残る間は終了しない。
// 4. Cancel・無関係な応答・待機解除は終了しない。再確認の終了は一度だけ。
// 5. 実アプリ・アカウント・認証ストア・ネットワークには一切触れない。

function pinnedSource() {
  const plist = readFileSync('/Applications/Claude.app/Contents/Info.plist', 'utf8');
  const version = plist.match(/<key>CFBundleShortVersionString<\/key>\s*<string>([^<]+)<\/string>/)?.[1];
  assert.equal(version, PIN.version, 'unreviewed desktop version');
  const archive = readFileSync('/Applications/Claude.app/Contents/Resources/app.asar');
  assert.equal(createHash('sha256').update(archive).digest('hex'), PIN.sha256,
    'unreviewed desktop archive');
  const headerLength = archive.readUInt32LE(12);
  assert(headerLength > 0 && headerLength <= 16 * 1024 * 1024, 'invalid ASAR header');
  const header = JSON.parse(archive.subarray(16, 16 + headerLength).toString('utf8'));
  let entry = header;
  for (const segment of PIN.chunk.split('/')) entry = entry.files[segment];
  const offset = 8 + archive.readUInt32LE(4) + Number(entry.offset);
  assert(Number.isSafeInteger(offset) && Number.isSafeInteger(entry.size), 'invalid ASAR entry');
  assert(offset >= 16 + headerLength && offset + entry.size <= archive.length, 'invalid ASAR bounds');
  return archive.subarray(offset, offset + entry.size).toString('utf8');
}

// この固定版で抽出する関数には regexp literal / template interpolation がない。
// 対応範囲を増やして曖昧に解析する代わりに未対応構文では停止する。
function extractFunction(source, name) {
  const escaped = name.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const matches = [...source.matchAll(new RegExp(`(?:async )?function ${escaped}\\(`, 'g'))];
  assert.equal(matches.length, 1, 'vendor function is not unique');
  const start = matches[0].index;
  const opening = source.indexOf('{', start);
  let depth = 0;
  let quote = null;
  for (let i = opening; i < source.length; i += 1) {
    const char = source[i];
    if (quote) {
      if (char === '\\') { i += 1; continue; }
      if (char === quote) quote = null;
      if (quote === '`' && char === '$' && source[i + 1] === '{') {
        throw Error('unsupported vendor template interpolation');
      }
      continue;
    }
    if (char === '"' || char === "'" || char === '`') { quote = char; continue; }
    if (char === '/' && (source[i + 1] === '/' || source[i + 1] === '*')) {
      throw Error('unsupported vendor comment syntax');
    }
    if (char === '{') depth += 1;
    if (char === '}' && --depth === 0) return source.slice(start, i + 1);
  }
  throw Error('unterminated vendor function');
}

function startupPrefix(source) {
  const start = source.indexOf(START);
  assert(start >= 0 && source.indexOf(START, start + 1) === -1, 'startup chain is not unique');
  const end = source.indexOf(INSTALLED, start);
  assert(end > start && end - start < 2048, 'startup guard installation not found');
  // ベンダーの登録直後で止める。後続の Dock・保存領域等の処理は実行しない。
  return `${source.slice(start, end + INSTALLED.length)};}))`;
}

class Clock {
  now = 0;
  next = 1;
  timers = new Map();
  schedule(callback, delay, interval = false) {
    assert(Number.isFinite(delay) && delay > 0, 'invalid fake timer');
    const handle = { id: this.next++, unref() {} };
    this.timers.set(handle.id, { handle, callback, at: this.now + delay, interval: interval ? delay : 0 });
    return handle;
  }
  clear = (handle) => { this.timers.delete(handle?.id); };
  async tick(delta) {
    const until = this.now + delta;
    for (;;) {
      const next = [...this.timers.values()]
        .filter((timer) => timer.at <= until)
        .sort((left, right) => left.at - right.at || left.handle.id - right.handle.id)[0];
      if (!next) break;
      this.now = next.at;
      if (next.interval) next.at += next.interval;
      else this.timers.delete(next.handle.id);
      next.callback();
      await settle();
    }
    this.now = until;
    await settle();
  }
}

async function settle() {
  // 起動 prefix の複数段の Promise chain を実時間なしで drain する。
  for (let i = 0; i < 32; i += 1) await Promise.resolve();
}

class Manager extends EventEmitter {
  constructor(kind = 'code') { super(); this.kind = kind; this.tasks = []; }
  add(activity, unattended = false) {
    this.tasks.push({ sessionId: `${this.kind}-${this.tasks.length}`, kind: this.kind, activity, unattended });
  }
  countRunningSessions(options) {
    return this.tasks.filter((task) => !options?.unattended || task.unattended).length;
  }
  listRunningSessions() { return this.tasks; }
  countRunningSessionsByActivity() {
    return Object.fromEntries(['turn', 'held_input', 'background_task', 'loop',
      'remote_client', 'violin_bow', 'lanyard'].map((activity) => [activity,
      this.tasks.filter((task) => task.activity === activity).length]));
  }
  idle(activity, event = 'session_updated') {
    this.tasks = this.tasks.filter((task) => task.activity !== activity);
    this.emit('event', { type: event });
  }
}

const source = pinnedSource();
const functions = FUNCTIONS.map((name) => extractFunction(source, name)).join('\n');
const prefix = startupPrefix(source);

function sandbox({ loading = true, view = true, dialogResponse = 1 } = {}) {
  const clock = new Clock();
  const code = new Manager();
  const cowork = new Manager('cowork');
  const webContents = new EventEmitter();
  webContents.isLoading = () => loading;
  const state = { guard: null, dialogs: [], quitCallbacks: 0, permittedQuits: 0, installedAt: null };
  const context = vm.createContext({
    setTimeout: (callback, delay) => clock.schedule(callback, delay),
    setInterval: (callback, delay) => clock.schedule(callback, delay, true),
    clearTimeout: clock.clear,
    clearInterval: clock.clear,
    Date: { now: () => clock.now },
    N: { info() {}, error() {} },
    Q() {},
    X: () => ({ formatMessage: ({ defaultMessage }) => defaultMessage }),
    P: () => 'fake',
    B: undefined,
    V: view ? { webContents } : undefined,
    AR: () => false,
    ZZt: () => [], // リモート parking 分岐は対象外。未知の分岐では停止。
    dQt: () => { throw Error('unexpected remote parking branch'); },
    ZOn: (guard) => { state.guard = guard; state.installedAt = clock.now; },
    a: {
      app: { quit: () => { state.quitCallbacks += 1; if (!state.guard()) state.permittedQuits += 1; } },
      dialog: { showMessageBox: async (...args) => {
        state.dialogs.push(args.at(-1));
        return { response: dialogResponse };
      } },
    },
    require: (name) => {
      if (name === './index.chunk-CFJssAFf.js') return { claudeCodeSessionManager: code };
      if (name === './index.chunk-CB6cNSrW.js') return { t: { localAgentModeSessionManager: cowork } };
      if (name === './index.chunk-CyQG_Vsp.js') return { n: { startCoworkDeletionOutbox() {} } };
      throw Error('unexpected vendor module');
    },
  }, { codeGeneration: { strings: false, wasm: false } });
  vm.runInContext(`var nQt,sQt=()=>false,cQt=false,eS=false,tS,nS;\n${functions}\n`
    + 'var tQt={installQuitSessionGuard:lQt};'
    + 'globalThis.api={install:lQt,pending:oQt,cancel:$x,schedule:aQt};', context,
  { timeout: 1000, filename: 'pinned-vendor-functions' });
  return {
    state, clock, code, cowork,
    api: context.api,
    start: () => vm.runInContext(prefix, context, { timeout: 1000, filename: 'pinned-startup-prefix' }),
    loaded() { loading = false; webContents.emit('did-finish-load'); },
    createView() { context.V = { webContents }; },
    loadListeners: () => webContents.listenerCount('did-finish-load'),
    install() { context.api.install([code, cowork], () => false); },
  };
}

const tests = [];
function test(name, run) { tests.push({ name, run }); }

// RED 用: 旧仮説をそのまま assertion にし、失敗を外側で握りつぶさない。
if (ARGS[0] === '--assert-five-second-readiness') {
  const fake = sandbox();
  fake.start();
  await fake.clock.tick(5001);
  try {
    assert.equal(typeof fake.state.guard, 'function', 'five-second readiness premise');
    throw Error('negative control did not reproduce');
  } catch (error) {
    if (!(error instanceof assert.AssertionError)) throw error;
    process.stderr.write('RED: five seconds elapsed but the quit guard is still absent (loading renderer).\n');
    process.exitCode = 1;
  }
} else {
  test('late renderer: five seconds do not establish guard readiness', async () => {
    const fake = sandbox();
    fake.start();
    await fake.clock.tick(5001);
    assert.equal(fake.state.guard, null);
    assert.equal(fake.state.installedAt, null);
    fake.loaded();
    await settle();
    assert.equal(fake.clock.timers.size, 0);
    assert.equal(typeof fake.state.guard, 'function');
    assert.equal(fake.state.installedAt, 5001);
    assert.equal(fake.state.quitCallbacks, 0);
  });

  test('loaded renderer: first 100 ms poll still precedes guard installation', async () => {
    const fake = sandbox({ view: false });
    fake.start();
    await fake.clock.tick(50);
    fake.createView();
    fake.loaded();
    await settle();
    assert.equal(fake.state.guard, null);
    await fake.clock.tick(49);
    assert.equal(fake.state.guard, null);
    await fake.clock.tick(1);
    assert.equal(fake.state.installedAt, 100);
  });

  test('60 second readiness timeout is caught before installation', async () => {
    const fake = sandbox({ view: false });
    fake.start();
    await fake.clock.tick(59_999);
    assert.equal(fake.state.guard, null);
    await fake.clock.tick(1);
    assert.equal(fake.state.installedAt, 60_000);
    assert.equal(fake.clock.timers.size, 0);
  });

  test('loading-view timeout removes its did-finish-load listener', async () => {
    const fake = sandbox();
    fake.start();
    await fake.clock.tick(100);
    assert.equal(fake.loadListeners(), 1);
    await fake.clock.tick(59_900);
    assert.equal(fake.loadListeners(), 0);
    assert.equal(fake.state.installedAt, 60_000);
    assert.equal(fake.clock.timers.size, 0);
    fake.loaded();
    await settle();
    assert.equal(fake.state.installedAt, 60_000);
  });

  test('already loaded view registers asynchronously, without a five second requirement', async () => {
    const fake = sandbox({ loading: false });
    fake.start();
    assert.equal(fake.state.guard, null);
    await settle();
    assert.equal(fake.state.installedAt, 0);
  });

  for (const activity of ['turn', 'held_input', 'background_task']) {
    test(`exact Wait defers fake ${activity} work until idle`, async () => {
      const fake = sandbox();
      fake.code.add(activity);
      fake.install();
      assert.equal(fake.state.guard(), true);
      await settle();
      assert.deepEqual(Array.from(fake.state.dialogs[0].buttons), ['Quit anyway', 'Wait for Claude', 'Cancel']);
      assert.equal(fake.state.dialogs[0].defaultId, 1);
      assert.equal(fake.state.dialogs[0].cancelId, 2);
      assert.equal(fake.api.pending(), true);
      await fake.clock.tick(60_000);
      assert.equal(fake.state.quitCallbacks, 0);
      fake.code.idle(activity);
      assert.equal(fake.state.quitCallbacks, 1);
      assert.equal(fake.state.permittedQuits, 1);
      assert.equal(fake.api.pending(), false);
      fake.code.emit('event', { type: 'session_updated' });
      await fake.clock.tick(120_000);
      assert.equal(fake.state.quitCallbacks, 1);
      assert.equal(fake.code.listenerCount('event'), 0);
      assert.equal(fake.clock.timers.size, 0);
    });
  }

  test('Wait requires Code and Cowork work to be idle, including unattended work', async () => {
    const fake = sandbox();
    fake.code.add('turn');
    fake.code.add('held_input');
    fake.cowork.add('background_task', true);
    fake.install();
    assert.equal(fake.state.guard(), true);
    await settle();
    fake.code.idle('turn');
    fake.code.idle('held_input');
    assert.equal(fake.state.quitCallbacks, 0);
    fake.cowork.idle('background_task');
    assert.equal(fake.state.quitCallbacks, 1);
  });

  test('unattended count can hold Wait even when the normal count is zero', async () => {
    const fake = sandbox();
    fake.code.add('background_task', true);
    const count = fake.code.countRunningSessions.bind(fake.code);
    fake.code.countRunningSessions = (options) => options?.unattended ? count(options) : 0;
    let callbacks = 0;
    fake.api.schedule([fake.code], () => { callbacks += 1; });
    assert.equal(fake.api.pending(), true);
    await fake.clock.tick(60_000);
    assert.equal(callbacks, 0);
    fake.code.idle('background_task');
    assert.equal(callbacks, 1);
    assert.equal(fake.api.pending(), false);
  });

  test('repeated quit requests during the warning create only one pending Wait', async () => {
    const fake = sandbox();
    fake.code.add('turn');
    fake.install();
    assert.equal(fake.state.guard(), true);
    assert.equal(fake.state.guard(), true);
    await settle();
    assert.equal(fake.state.dialogs.length, 1);
    assert.equal(fake.code.listenerCount('event'), 1);
    fake.code.idle('turn');
    assert.equal(fake.state.quitCallbacks, 1);
  });

  test('message-only events do not complete Wait; periodic recheck does', async () => {
    const fake = sandbox();
    fake.code.add('turn');
    fake.install();
    fake.state.guard();
    await settle();
    fake.code.idle('turn', 'message');
    fake.code.emit('event', { type: 'side_chat_assistant_delta' });
    assert.equal(fake.state.quitCallbacks, 0);
    await fake.clock.tick(59_999);
    assert.equal(fake.state.quitCallbacks, 0);
    await fake.clock.tick(1);
    assert.equal(fake.state.quitCallbacks, 1);
  });

  for (const response of [2, 99]) {
    test(`dialog response ${response} does not schedule a quit`, async () => {
      const fake = sandbox({ dialogResponse: response });
      fake.code.add('turn');
      fake.install();
      assert.equal(fake.state.guard(), true);
      await settle();
      assert.equal(fake.api.pending(), false);
      fake.code.idle('turn');
      await fake.clock.tick(60_000);
      assert.equal(fake.state.quitCallbacks, 0);
    });
  }

  test('explicit pending-Wait cancellation removes callbacks and timers', async () => {
    const fake = sandbox();
    fake.code.add('background_task');
    fake.install();
    fake.state.guard();
    await settle();
    assert.equal(fake.api.pending(), true);
    fake.api.cancel();
    fake.code.idle('background_task');
    await fake.clock.tick(60_000);
    assert.equal(fake.state.quitCallbacks, 0);
    assert.equal(fake.code.listenerCount('event'), 0);
    assert.equal(fake.clock.timers.size, 0);
  });

  test('already idle guard allows the fake caller without a warning', async () => {
    const fake = sandbox();
    fake.install();
    assert.equal(fake.state.guard(), false);
    await settle();
    assert.equal(fake.state.dialogs.length, 0);
    assert.equal(fake.state.quitCallbacks, 0);
  });

  for (const { name, run } of tests) {
    await run();
    process.stdout.write(`PASS ${name}\n`);
  }
  process.stdout.write(`${tests.length} isolated checks passed; desktop ${PIN.version}, archive ${PIN.sha256}.\n`);
  process.stdout.write('LIMIT: fake managers/events only. No real task-before-guard ordering, native Accessibility, process freshness, or safe automatic quit has been established.\n');
}
