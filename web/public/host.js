export function createHost(socket, tabId, clipboard, storage) {
  let next = 0;
  let view;
  let ready = false;
  let initialized = false;
  let stopped = false;
  const pending = new Map();
  const serverSubscriptions = new Map();
  const clientSubscriptions = new Map();
  const queued = [];
  const configKey = 'lean4-ide.infoview.config';
  const opened = new Promise((resolve, reject) => {
    if (socket.readyState === 1) resolve();
    else {
      socket.addEventListener('open', resolve, { once: true });
      socket.addEventListener('error', () => reject(new Error('Infoview disconnected')), { once: true });
      socket.addEventListener('close', () => reject(new Error('Infoview disconnected')), { once: true });
    }
  });
  opened.catch(() => {});

  async function call(name, args) {
    await opened;
    if (stopped || socket.readyState !== 1) throw new Error('Infoview disconnected');
    const id = `${tabId}:${++next}`;
    return new Promise((resolve, reject) => {
      pending.set(id, { resolve, reject });
      try { socket.send(JSON.stringify({ id, call: name, args })); }
      catch (error) { pending.delete(id); reject(error); }
    });
  }

  async function deliver(name, args) {
    if (!ready) { queued.push([name, args]); return; }
    if (name === 'gotServerNotification' && serverSubscriptions.get(args[0]) > 0) {
      await view.gotServerNotification(...args);
    } else if (name === 'sentClientNotification' && clientSubscriptions.get(args[0]) > 0) {
      await view.sentClientNotification(...args);
    } else if (name === 'changedCursorLocation') {
      const loc = args[0] ?? undefined;
      if (!initialized && loc) {
        initialized = true;
        await view.initialize(loc);
      } else if (initialized) await view.changedCursorLocation(loc);
    }
  }

  socket.addEventListener('message', event => {
    let m;
    try { m = JSON.parse(event.data); }
    catch { socket.close(); return; }
    if (!m || typeof m !== 'object' || Array.isArray(m)) return;
    if (typeof m.id === 'string') {
      const waiter = pending.get(m.id);
      if (!waiter) return;
      pending.delete(m.id);
      if (Object.hasOwn(m, 'error')) {
        waiter.reject(Object.assign(new Error(m.error?.message ?? 'Proxy error'), m.error));
      } else waiter.resolve(m.result);
    } else if (['gotServerNotification', 'sentClientNotification', 'changedCursorLocation'].includes(m.event) && Array.isArray(m.args)) {
      deliver(m.event, m.args).catch(console.error);
    }
  });
  function disconnect() {
    if (stopped) return;
    stopped = true;
    for (const waiter of pending.values()) waiter.reject(new Error('Infoview disconnected'));
    pending.clear();
    if (view) view.serverStopped({ message: 'Infoview disconnected. Reopen it from Zed.', reason: 'The proxy connection closed.' }).catch(console.error);
  }
  socket.addEventListener('close', disconnect);
  socket.addEventListener('error', disconnect);

  function subscribe(counts, method) { counts.set(method, (counts.get(method) ?? 0) + 1); }
  function unsubscribe(counts, method) {
    const count = counts.get(method) ?? 0;
    if (count <= 1) counts.delete(method);
    else counts.set(method, count - 1);
  }
  async function apply(name, args) {
    const result = await call(name, args);
    if (result?.applied !== true) throw new Error(result?.failureReason ?? 'Zed did not apply the edit');
  }

  const editorApi = {
    async saveConfig(config) {
      storage.setItem(configKey, JSON.stringify(config));
      await view.changedInfoviewConfig(config);
    },
    sendClientRequest: (uri, method, params, _options) => call('sendClientRequest', [uri, method, params]),
    async sendClientNotification(uri, method, params) { await call('sendClientNotification', [uri, method, params]); },
    async subscribeServerNotifications(method) {
      subscribe(serverSubscriptions, method);
      const cached = await call('subscribeServerNotifications', [method]);
      for (const params of cached) await deliver('gotServerNotification', [method, params]);
    },
    async unsubscribeServerNotifications(method) { unsubscribe(serverSubscriptions, method); },
    async subscribeClientNotifications(method) { subscribe(clientSubscriptions, method); },
    async unsubscribeClientNotifications(method) { unsubscribe(clientSubscriptions, method); },
    copyToClipboard: text => clipboard.writeText(text),
    insertText: (text, kind, pos) => apply('insertText', [text, kind, pos ?? null]),
    applyEdit: edit => apply('applyEdit', [edit]),
    async showDocument(params) {
      const result = await call('showDocument', [params]);
      if (result?.success !== true) throw new Error('Zed did not open the location');
    },
    async restartFile(uri) { await call('restartFile', [uri]); },
    createRpcSession: uri => call('createRpcSession', [uri]),
    async closeRpcSession(sessionId) { await call('closeRpcSession', [sessionId]); },
  };

  async function start(api, defaults) {
    view = api;
    const hello = await call('hello', []);
    if (!hello.initializeResult) throw new Error('Open the infoview from Zed after Lean starts.');
    if (hello.location) { initialized = true; await view.initialize(hello.location); }
    await view.serverRestarted(hello.initializeResult);
    let config = defaults;
    try {
      const saved = JSON.parse(storage.getItem(configKey));
      if (saved && typeof saved === 'object' && !Array.isArray(saved)) config = { ...defaults, ...saved };
    } catch { /* Ignore invalid saved preferences. */ }
    await view.changedInfoviewConfig(config);
    ready = true;
    for (const [name, args] of queued.splice(0)) await deliver(name, args);
  }
  return { editorApi, start };
}
