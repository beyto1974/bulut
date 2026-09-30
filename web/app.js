(() => {
  'use strict';

  const config = JSON.parse(document.documentElement.dataset.config || '{}');
  const $ = (id) => document.getElementById(id);

  // ---------- small helpers ----------

  function h(tag, props, ...kids) {
    const el = document.createElement(tag);
    for (const [k, v] of Object.entries(props || {})) {
      if (v === undefined || v === null || v === false) continue;
      if (k === 'class') el.className = v;
      else if (k === 'text') el.textContent = v;
      else if (k.startsWith('on')) el.addEventListener(k.slice(2), v);
      else el.setAttribute(k, v === true ? '' : String(v));
    }
    for (const kid of kids.flat()) {
      if (kid === null || kid === undefined || kid === false) continue;
      el.append(kid.nodeType ? kid : document.createTextNode(String(kid)));
    }
    return el;
  }

  function fmtSize(bytes) {
    if (bytes < 1000) return `${bytes} B`;
    const units = ['KB', 'MB', 'GB', 'TB'];
    let v = bytes / 1000;
    let i = 0;
    while (v >= 1000 && i < units.length - 1) { v /= 1000; i++; }
    return `${v.toFixed(1)} ${units[i]}`;
  }

  function fmtTime(iso) {
    if (!iso) return 'unknown';
    const d = new Date(iso);
    if (Number.isNaN(d.getTime())) return 'unknown';
    const p = (n) => String(n).padStart(2, '0');
    return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
  }

  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

  function toast(message, isError) {
    const el = h('div', { class: 'toast' + (isError ? ' err' : ''), text: message });
    $('toasts').append(el);
    setTimeout(() => el.remove(), isError ? 6000 : 2500);
  }

  class ApiError extends Error {
    constructor(message, status) { super(message); this.status = status; }
  }

  async function api(method, path, body) {
    const opts = { method, headers: {} };
    if (body !== undefined) {
      opts.headers['content-type'] = 'application/json';
      opts.body = JSON.stringify(body);
    }
    const res = await fetch(path, opts);
    if (res.status === 204) return null;
    const text = await res.text();
    let data = null;
    try { data = text ? JSON.parse(text) : null; } catch (_) { /* plain text error */ }
    if (!res.ok) throw new ApiError((data && data.error) || text || `Request failed (${res.status})`, res.status);
    return data;
  }

  async function copyText(text) {
    try {
      await navigator.clipboard.writeText(text);
    } catch (_) {
      const ta = h('textarea', { 'aria-hidden': 'true' });
      ta.value = text;
      document.body.append(ta);
      ta.select();
      try { document.execCommand('copy'); } catch (_) { /* nothing more to try */ }
      ta.remove();
    }
    toast('Link copied');
  }

  // ---------- chrome shared by both pages ----------

  function chrome() {
    if (config.env && config.env !== 'production') {
      const badge = $('env-badge');
      badge.textContent = config.env;
      badge.hidden = false;
    }
    $('footer-version').textContent = `bulut v${config.version || '?'}` +
      (config.env && config.env !== 'production' ? ` (${config.env})` : '');
  }

  // ---------- home ----------

  function home() {
    const input = h('input', {
      type: 'text', id: 'code-input', name: 'code', maxlength: config.codeLength, autocomplete: 'off',
      autocapitalize: 'none', spellcheck: 'false', placeholder: 'k7m3q', 'aria-label': 'Session code',
    });
    const form = h('form', {
      onsubmit: (e) => {
        e.preventDefault();
        const code = input.value.trim().toLowerCase();
        if (code) location.href = `/${encodeURIComponent(code)}`;
      },
    }, input, h('button', { class: 'btn primary', type: 'submit', text: 'Open session' }));
    const create = h('button', {
      class: 'btn', type: 'button', id: 'new-session', text: 'New session',
      onclick: async () => {
        try {
          const s = await api('POST', '/api/s', {});
          location.href = `/${s.code}`;
        } catch (e) { toast(e.message, true); }
      },
    });
    $('main').replaceChildren(h('section', { class: 'home' },
      h('h1', { text: 'Share files with a short link' }),
      h('p', { class: 'muted', text: 'A session holds files for people and for development environments. Open one with its code or start a new one.' }),
      form,
      h('div', null, create),
    ));
    input.focus();
  }

  function notFound(message) {
    $('main').replaceChildren(h('section', { class: 'home' },
      h('h1', { text: 'Session not found' }),
      h('p', { class: 'muted', text: message || 'Check the code. Sessions are deleted after a period without activity.' }),
      h('div', null, h('a', { class: 'btn primary', href: '/', text: 'Back to start' })),
    ));
  }

  // ---------- session page ----------

  async function sessionPage(code) {
    let sess;
    try {
      sess = await api('GET', `/api/s/${code}`);
    } catch (e) {
      notFound(e.status === 404 ? null : e.message);
      return;
    }
    document.title = `${code} · Bulut` + (config.env && config.env !== 'production' ? ` (${config.env})` : '');

    const state = { items: [], selectedId: null, versions: [], versionId: null, queue: [], busy: false };
    const base = `/api/s/${code}`;

    // --- skeleton ---
    const descView = h('div', { class: 'desc panel', id: 'description-panel' });
    const tableBody = h('tbody');
    const table = h('div', { class: 'panel' },
      h('div', { class: 'table-wrap' },
        h('table', { 'aria-label': 'Files' },
          h('thead', null, h('tr', null,
            h('th', { text: 'Name' }), h('th', { text: 'Tags' }),
            h('th', { class: 'num', text: 'Size' }), h('th', { class: 'num', text: 'Uploaded' }))),
          tableBody)),
      h('div', { class: 'empty', id: 'empty', text: 'No files yet. Drop files on this page or choose them below.' }));
    const queueEl = h('div', { class: 'queue', id: 'queue' });
    const fileInput = h('input', { type: 'file', id: 'file-input', multiple: true, hidden: true });
    const drop = h('div', { class: 'drop', id: 'drop', tabindex: 0, role: 'button', 'aria-label': 'Choose files to upload' },
      'Drop files anywhere on this page, or click to choose. Uploading a name that exists adds a new version.');
    const upload = h('div', { class: 'upload panel' }, drop, fileInput, queueEl);
    const inspector = h('aside', { class: 'inspector panel', id: 'inspector', 'aria-live': 'polite' });

    const copyBtn = h('button', { class: 'btn primary', type: 'button', id: 'copy-link', text: 'Copy link', onclick: () => copyText(sess.url) });
    const qrBtn = h('button', { class: 'btn', type: 'button', id: 'show-qr', text: 'QR code', onclick: openQr });

    $('main').replaceChildren(
      h('div', { class: 'head' },
        h('div', null, h('div', { class: 'tag', text: 'Session' }), h('div', { class: 'code', id: 'session-code', text: code })),
        h('div', { class: 'link-box' }, h('span', { id: 'session-url', text: sess.url }), copyBtn, qrBtn)),
      descView,
      h('div', { class: 'main' }, h('div', { class: 'stack' }, table, upload), inspector),
    );

    // --- description ---
    function renderDescription(editing) {
      const expiry = h('div', { class: 'expiry', id: 'expiry',
        text: `This session is deleted on ${fmtTime(sess.expires_at)} unless someone opens it before then. Every visit, download or upload restarts the ${sess.idle_ttl_days} day timer.` });
      if (!editing) {
        descView.replaceChildren(
          h('span', { class: 'tag', text: 'Description' }),
          sess.description ? h('p', { id: 'description-text', text: sess.description }) : h('p', { class: 'muted', id: 'description-text', text: 'No description.' }),
          h('div', { class: 'actions' }, h('button', { class: 'btn', type: 'button', id: 'edit-description', text: 'Edit', onclick: () => renderDescription(true) })),
          expiry);
        return;
      }
      const ta = h('textarea', { id: 'description-input', maxlength: 2000, 'aria-label': 'Description' });
      ta.value = sess.description;
      descView.replaceChildren(
        h('span', { class: 'tag', text: 'Description' }), ta,
        h('div', { class: 'actions' },
          h('button', { class: 'btn primary', type: 'button', id: 'save-description', text: 'Save', onclick: async () => {
            try {
              const s = await api('PATCH', base, { description: ta.value });
              sess = s;
              renderDescription(false);
              toast('Description saved');
            } catch (e) { toast(e.message, true); }
          } }),
          h('button', { class: 'btn', type: 'button', text: 'Cancel', onclick: () => renderDescription(false) })),
        expiry);
      ta.focus();
    }

    // --- table ---
    function chips(tags) {
      return (tags || []).map((t) => h('span', { class: 'chip' + (t === 'latest' ? ' latest' : ''), text: t }));
    }

    function renderTable() {
      $('empty').hidden = state.items.length > 0;
      tableBody.replaceChildren(...state.items.map((it) => {
        const latest = it.latest;
        const tr = h('tr', { class: 'row-item', tabindex: 0, 'aria-selected': it.id === state.selectedId ? 'true' : 'false', 'data-id': it.id },
          h('td', { class: 'name' }, h('span', { class: 'icon', 'aria-hidden': 'true' }), it.name,
            it.note ? h('span', { class: 'dot', title: 'Has a note' }) : null,
            it.version_count > 1 ? h('span', { class: 'muted', text: ` · ${it.version_count} versions` }) : null),
          h('td', null, latest ? chips(latest.tags) : null),
          h('td', { class: 'num', text: latest ? fmtSize(latest.size) : '' }),
          h('td', { class: 'num', text: latest ? fmtTime(latest.uploaded_at) : '' }));
        const pick = () => select(it.id);
        tr.addEventListener('click', pick);
        tr.addEventListener('keydown', (e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); pick(); } });
        return tr;
      }));
    }

    // --- inspector ---
    function currentItem() { return state.items.find((i) => i.id === state.selectedId); }
    function currentVersion() { return state.versions.find((v) => v.id === state.versionId); }

    function renderInspector() {
      const it = currentItem();
      const ver = currentVersion();
      if (!it || !ver) {
        inspector.replaceChildren(h('div', { class: 'placeholder', text: 'Select a file to see its versions, tags and note.' }));
        return;
      }
      const tagInput = h('input', { type: 'text', id: 'tag-input', placeholder: 'Add tag', 'aria-label': 'Add tag', maxlength: 64 });
      tagInput.addEventListener('keydown', async (e) => {
        if (e.key !== 'Enter') return;
        e.preventDefault();
        const tag = tagInput.value.trim();
        if (!tag) return;
        try {
          await api('PUT', `${base}/versions/${ver.id}/tags/${encodeURIComponent(tag)}`);
          await refreshVersions();
          await reload();
        } catch (err) { toast(err.message, true); }
      });
      const tagChips = (ver.tags || []).map((t) => {
        const chip = h('span', { class: 'chip' + (t === 'latest' ? ' latest' : ''), text: t });
        if (t !== 'latest') {
          chip.append(h('button', { type: 'button', 'aria-label': `Remove tag ${t}`, text: '×', onclick: async () => {
            try {
              await api('DELETE', `${base}/versions/${ver.id}/tags/${encodeURIComponent(t)}`);
              await refreshVersions();
              await reload();
            } catch (err) { toast(err.message, true); }
          } }));
        }
        return chip;
      });
      const note = h('textarea', { id: 'note-input', maxlength: 4000, placeholder: 'Add a note for whoever opens this', 'aria-label': 'Note' });
      note.value = it.note || '';

      let armed = false;
      const del = h('button', { class: 'btn danger', type: 'button', id: 'delete-file', text: 'Delete file' });
      del.addEventListener('click', async () => {
        if (!armed) {
          armed = true;
          del.textContent = 'Click again to delete';
          del.classList.add('confirm');
          setTimeout(() => { armed = false; del.textContent = 'Delete file'; del.classList.remove('confirm'); }, 4000);
          return;
        }
        try {
          await api('DELETE', `${base}/nodes/${it.id}`);
          state.selectedId = null; state.versions = []; state.versionId = null;
          await reload();
          toast('File deleted');
        } catch (err) { toast(err.message, true); }
      });

      const versions = state.versions.map((v) => h('button', {
        class: 'ver', type: 'button', 'aria-pressed': v.id === state.versionId ? 'true' : 'false',
        onclick: () => { state.versionId = v.id; renderInspector(); },
      }, h('span', { text: `v${v.version}${v.is_latest ? ' · latest' : ''}` }), h('span', { text: `${fmtSize(v.size)} · ${fmtTime(v.uploaded_at)}` })));

      inspector.replaceChildren(
        h('div', { class: 'preview', 'aria-hidden': 'true', text: ver.content_type }),
        h('div', null, h('h2', { id: 'inspector-name', text: it.name }), h('div', { class: 'muted mono', text: `${code} / ${it.name}` })),
        h('dl', null,
          h('dt', { text: 'Version' }), h('dd', { text: `v${ver.version} of ${state.versions.length}` }),
          h('dt', { text: 'Size' }), h('dd', { text: fmtSize(ver.size) }),
          h('dt', { text: 'Created' }), h('dd', { id: 'meta-created', title: ver.client_created_at || '', text: fmtTime(ver.client_created_at) }),
          h('dt', { text: 'Uploaded' }), h('dd', { id: 'meta-uploaded', title: ver.uploaded_at, text: fmtTime(ver.uploaded_at) }),
          h('dt', { text: 'Type' }), h('dd', { text: ver.content_type })),
        state.versions.length > 1 ? h('div', null, h('div', { class: 'tag', text: 'Versions' }), h('div', { class: 'versions', id: 'versions' }, versions)) : null,
        h('div', null, h('div', { class: 'tag', text: `Tags on v${ver.version}` }), h('div', { class: 'tags', id: 'tags' }, tagChips, tagInput)),
        h('div', null, h('label', { class: 'tag', for: 'note-input', text: 'Note' }), note),
        h('div', { class: 'row' },
          h('button', { class: 'btn primary', type: 'button', id: 'save-note', text: 'Save note', onclick: async () => {
            try {
              const n = await api('PATCH', `${base}/nodes/${it.id}`, { note: note.value });
              it.note = n.note;
              renderTable();
              toast('Note saved');
            } catch (err) { toast(err.message, true); }
          } }),
          h('a', { class: 'btn', id: 'download', href: `${base}/versions/${ver.id}/download`, text: `Download v${ver.version}` }),
          del));
    }

    async function refreshVersions() {
      if (!state.selectedId) { state.versions = []; state.versionId = null; return; }
      state.versions = await api('GET', `${base}/nodes/${state.selectedId}/versions`);
      if (!state.versions.some((v) => v.id === state.versionId)) {
        state.versionId = state.versions.length ? state.versions[0].id : null;
      }
      renderInspector();
    }

    async function select(id) {
      state.selectedId = id;
      state.versionId = null;
      renderTable();
      try { await refreshVersions(); } catch (e) { toast(e.message, true); }
    }

    async function reload() {
      try {
        const listing = await api('GET', `${base}/files`);
        state.items = listing.items.filter((i) => i.kind === 'file');
      } catch (e) {
        toast(e.message, true);
        return;
      }
      if (state.selectedId && !state.items.some((i) => i.id === state.selectedId)) {
        state.selectedId = null; state.versions = []; state.versionId = null;
      }
      renderTable();
      renderInspector();
    }

    // --- uploads ---
    async function putPart(url, statusUrl, file, partSize, n) {
      for (let attempt = 1; attempt <= 4; attempt++) {
        try {
          const blob = file.slice((n - 1) * partSize, Math.min(n * partSize, file.size));
          const res = await fetch(url, { method: 'PUT', body: blob, headers: { 'content-type': 'application/octet-stream' } });
          if (res.ok) return;
          if (res.status >= 400 && res.status < 500) {
            let msg = `Upload refused (${res.status})`;
            try { msg = (await res.json()).error || msg; } catch (_) { /* keep default */ }
            const err = new Error(msg);
            err.fatal = true;
            throw err;
          }
        } catch (e) {
          if (e.fatal) throw e;
        }
        await sleep(700 * attempt);
        // After a failure, ask the server whether the part arrived before sending it again.
        try {
          const st = await api('GET', statusUrl);
          if (st.parts_received.includes(n)) return;
        } catch (_) { /* try the part again */ }
      }
      throw new Error('Connection lost. Try again.');
    }

    async function uploadFile(file, onProgress) {
      const init = await api('POST', `${base}/uploads`, {
        name: file.name,
        size: file.size,
        content_type: file.type || undefined,
        created_at: file.lastModified ? new Date(file.lastModified).toISOString() : undefined,
      });
      const statusUrl = `${base}/uploads/${init.upload_id}`;
      for (let n = 1; n <= init.parts_total; n++) {
        await putPart(`${statusUrl}/parts/${n}`, statusUrl, file, init.part_size, n);
        onProgress(n / init.parts_total);
      }
      onProgress(1);
      return api('POST', `${statusUrl}/complete`);
    }

    async function processQueue() {
      if (state.busy) return;
      state.busy = true;
      while (state.queue.length) {
        const job = state.queue.shift();
        job.status.textContent = 'Uploading';
        try {
          const stored = await uploadFile(job.file, (f) => { job.bar.style.width = `${Math.round(f * 100)}%`; });
          job.status.textContent = `Done, v${stored.version.version}`;
          await reload();
          await select(stored.node.id);
        } catch (e) {
          job.status.textContent = e.message;
          job.status.classList.add('err');
        }
      }
      state.busy = false;
    }

    function enqueue(files) {
      for (const file of files) {
        const bar = h('i');
        const status = h('span', { class: 'q-status', text: 'Waiting' });
        queueEl.append(h('div', { class: 'q-item' },
          h('div', { class: 'q-head' }, h('span', { text: `${file.name} (${fmtSize(file.size)})` }), status),
          h('div', { class: 'progress' }, bar)));
        state.queue.push({ file, bar, status });
      }
      processQueue();
    }

    function acceptDrop(dt) {
      const files = [];
      const items = [...(dt.items || [])];
      let skipped = false;
      if (items.length && items[0].webkitGetAsEntry) {
        for (const item of items) {
          const entry = item.webkitGetAsEntry();
          if (entry && entry.isDirectory) { skipped = true; continue; }
          const f = item.getAsFile();
          if (f) files.push(f);
        }
      } else {
        files.push(...dt.files);
      }
      if (skipped) toast('Folders are not supported yet, only files were added.', true);
      if (files.length) enqueue(files);
    }

    drop.addEventListener('click', () => fileInput.click());
    drop.addEventListener('keydown', (e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); fileInput.click(); } });
    fileInput.addEventListener('change', () => { enqueue([...fileInput.files]); fileInput.value = ''; });
    window.addEventListener('dragover', (e) => { e.preventDefault(); drop.classList.add('over'); });
    window.addEventListener('dragleave', (e) => { if (!e.relatedTarget) drop.classList.remove('over'); });
    window.addEventListener('drop', (e) => { e.preventDefault(); drop.classList.remove('over'); if (e.dataTransfer) acceptDrop(e.dataTransfer); });

    // --- QR dialog ---
    const dialog = $('qr-dialog');
    function openQr() {
      $('qr-image').src = `${base}/qr.svg`;
      $('qr-url').textContent = sess.url;
      $('qr-download').href = `${base}/qr.svg?download=1`;
      dialog.showModal();
    }
    $('qr-close').addEventListener('click', () => dialog.close());
    dialog.addEventListener('click', (e) => { if (e.target === dialog) dialog.close(); });

    // Refresh when the tab is opened again. No polling: that would keep an unattended session alive.
    document.addEventListener('visibilitychange', () => { if (document.visibilityState === 'visible') reload(); });

    renderDescription(false);
    await reload();
  }

  // ---------- start ----------

  chrome();
  const path = decodeURIComponent(location.pathname).replace(/^\/+|\/+$/g, '');
  if (!path) {
    home();
  } else {
    sessionPage(path.toLowerCase());
  }
})();
