import { renderInfoview, defaultInfoviewConfig } from '@leanprover/infoview';
import { createHost } from './host.js';

const status = document.getElementById('status');
const copy = document.getElementById('copy-comment');
const url = new URL('/ws', window.location.href);
url.protocol = 'ws:';
url.search = window.location.search;
const socket = new WebSocket(url);
const host = createHost(socket, crypto.randomUUID(), navigator.clipboard, localStorage);
const view = renderInfoview(host.editorApi, document.getElementById('infoview'));
socket.addEventListener('close', () => {
  status.textContent = 'Infoview disconnected. Reopen it from Zed.';
  copy.disabled = true;
});
copy.addEventListener('click', () => {
  view.requestedAction({ kind: 'copyToComment' }).catch(error => { status.textContent = error.message; });
});
try {
  await host.start(view, defaultInfoviewConfig);
  status.textContent = '';
  copy.disabled = false;
} catch (error) {
  socket.close();
  status.textContent = error.message;
}
