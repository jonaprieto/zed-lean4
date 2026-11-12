import test from 'node:test';
import assert from 'node:assert/strict';
import { createHost } from '../public/host.js';

class Socket extends EventTarget {
  readyState = 1;
  sent = [];
  send(text) { this.sent.push(JSON.parse(text)); }
  emit(message) { this.dispatchEvent(Object.assign(new Event('message'), { data: JSON.stringify(message) })); }
  close() { this.readyState = 3; this.dispatchEvent(new Event('close')); }
}
const turn = () => new Promise(resolve => setImmediate(resolve));
const loc = { uri: 'file:///F.lean', range: { start: { line: 1, character: 2 }, end: { line: 1, character: 2 } } };
async function setup() {
  const socket = new Socket();
  const events = [];
  const storage = { getItem: () => null, setItem: (key, value) => events.push(['storage', key, value]) };
  const host = createHost(socket, 'tab', { writeText: async text => events.push(['clipboard', text]) }, storage);
  const view = Object.fromEntries(['initialize', 'serverRestarted', 'serverStopped', 'changedInfoviewConfig', 'gotServerNotification', 'sentClientNotification', 'changedCursorLocation', 'runTestScript'].map(name => [name, async (...args) => events.push([name, ...args])]));
  const started = host.start(view, { debounceTime: 50 });
  await turn();
  socket.emit({ id: socket.sent.at(-1).id, result: { location: loc, initializeResult: { capabilities: {} } } });
  await started;
  return { socket, events, api: host.editorApi };
}

test('initializes once, correlates replies, preserves errors and ignores unknown events', async () => {
  const { socket, events, api } = await setup();
  const first = api.sendClientRequest(loc.uri, 'one', {});
  const second = api.sendClientRequest(loc.uri, 'two', {});
  const rejected = assert.rejects(second, e => e.code === -32900 && e.data.detail === 1);
  await turn();
  const [a, b] = socket.sent.slice(-2);
  assert.equal(a.id, 'tab:2');
  socket.emit({ id: 'other:2', result: 7 });
  socket.emit({ id: b.id, error: { code: -32900, message: 'expired', data: { detail: 1 } } });
  socket.emit({ id: a.id, result: null });
  assert.equal(await first, null); await rejected;
  socket.emit({ event: 'runTestScript', args: ['throw 1'] });
  socket.emit({ event: 'changedCursorLocation', args: [loc] });
  await turn();
  assert.equal(events.filter(e => e[0] === 'initialize').length, 1);
  assert.equal(events.filter(e => e[0] === 'runTestScript').length, 0);
  const pending = api.sendClientRequest(loc.uri, 'pending', {});
  const closed = assert.rejects(pending, /disconnected/);
  await turn(); socket.close(); await closed;
  assert.ok(events.some(e => e[0] === 'serverStopped'));
});

test('subscription counts filter events and replay caches', async () => {
  const { socket, events, api } = await setup();
  const method = 'textDocument/publishDiagnostics';
  for (const version of [1, 2]) {
    const subscribed = api.subscribeServerNotifications(method);
    await turn(); socket.emit({ id: socket.sent.at(-1).id, result: [{ uri: loc.uri, version }] });
    await subscribed;
  }
  await api.unsubscribeServerNotifications(method);
  socket.emit({ event: 'gotServerNotification', args: [method, { version: 3 }] });
  await api.unsubscribeServerNotifications(method);
  socket.emit({ event: 'gotServerNotification', args: [method, { version: 4 }] });
  await api.subscribeClientNotifications('textDocument/didChange');
  socket.emit({ event: 'sentClientNotification', args: ['textDocument/didChange', {}] });
  socket.emit({ event: 'sentClientNotification', args: ['ignored', {}] });
  await turn();
  assert.deepEqual(events.filter(e => e[0] === 'gotServerNotification').map(e => e[2].version), [1, 2, 3]);
  assert.equal(events.filter(e => e[0] === 'sentClientNotification').length, 1);
});

test('editor operations use exact args and reject denied edits', async () => {
  const { socket, events, api } = await setup();
  const edit = api.insertText('text', 'above');
  const denied = assert.rejects(edit, /read only/);
  await turn();
  assert.deepEqual(socket.sent.at(-1).args, ['text', 'above', null]);
  socket.emit({ id: socket.sent.at(-1).id, result: { applied: false, failureReason: 'read only' } });
  await denied;
  await api.copyToClipboard('hello');
  await api.saveConfig({ debounceTime: 80 });
  assert.ok(events.some(e => e[0] === 'clipboard' && e[1] === 'hello'));
  assert.ok(events.some(e => e[0] === 'changedInfoviewConfig' && e[1].debounceTime === 80));
});
