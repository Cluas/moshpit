// Docs search — client side, because there is no server to ask.
//
// The same constraint that shapes the app shapes this: nothing in the middle.
// The index is a static JSON file built by scripts/build-docs.py, fetched once
// on first use (not on page load — most readers never search), and matched with
// a plain scoring pass. No library, no analytics, no query leaves the page.
(function () {
  const modal = document.getElementById('dsearch');
  const input = document.getElementById('dq');
  const results = document.getElementById('dres');
  if (!modal || !input || !results) return;

  const isZh = document.documentElement.lang.startsWith('zh');
  let index = null;
  let loading = null;

  function load() {
    if (index) return Promise.resolve(index);
    if (!loading) {
      loading = fetch('/docs-index.json')
        .then((r) => r.json())
        .then((data) => {
          // Show the reader's own language first; a Chinese reader searching
          // "密钥" should not have to scroll past the English page to find it.
          index = data.filter((e) => e.l === (isZh ? 'zh' : 'en'));
          return index;
        })
        .catch(() => (index = []));
    }
    return loading;
  }

  function score(entry, q) {
    const title = entry.t.toLowerCase();
    const body = entry.b.toLowerCase();
    if (title === q) return 100;
    if (title.includes(q)) return 60;
    if (body.includes(q)) return 20;
    return 0;
  }

  function snippet(entry, q) {
    const at = entry.b.toLowerCase().indexOf(q);
    if (at < 0) return entry.b.slice(0, 120) + '…';
    const from = Math.max(0, at - 45);
    return (from ? '…' : '') + entry.b.slice(from, from + 150) + '…';
  }

  function render(q) {
    const query = q.trim().toLowerCase();
    if (!query) {
      results.innerHTML = '';
      return;
    }
    if (!index) {
      // Not "nothing matched" — we have not looked yet. Saying no to a query
      // with plenty of hits is the one answer a search must never give.
      results.innerHTML =
        '<p class="dnohit">' + (isZh ? '正在载入索引…' : 'Loading the index…') + '</p>';
      return;
    }
    const hits = index
      .map((e) => ({ e, s: score(e, query) }))
      .filter((x) => x.s > 0)
      .sort((a, b) => b.s - a.s)
      .slice(0, 8);
    if (!hits.length) {
      results.innerHTML =
        '<p class="dnohit">' + (isZh ? '没有匹配的页面。' : 'Nothing matched.') + '</p>';
      return;
    }
    results.innerHTML = hits
      .map(
        ({ e }) =>
          '<a href="' + e.u + '"><b>' + e.t + '</b><span>' + snippet(e, query) + '</span></a>'
      )
      .join('');
  }

  function open() {
    modal.hidden = false;
    // The element carries an inline display:none so that a stale stylesheet can
    // never leave it covering the page. Opening therefore has to set display
    // here rather than rely on the class alone.
    modal.style.display = 'flex';
    input.value = '';
    results.innerHTML = '';
    // Re-render once the index lands. Readers start typing before the fetch
    // resolves, and without this they are shown "nothing matched" for a query
    // that has plenty of hits, with no keystroke left to correct it.
    load().then(() => {
      if (!modal.hidden) render(input.value);
      input.focus();
    });
    input.focus();
  }

  function close() {
    modal.hidden = true;
    modal.style.display = 'none';
  }

  input.addEventListener('input', () => render(input.value));
  modal.addEventListener('click', (e) => {
    if (e.target === modal) close();
  });

  document.addEventListener('keydown', (e) => {
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'k') {
      e.preventDefault();
      modal.hidden ? open() : close();
    } else if (e.key === 'Escape' && !modal.hidden) {
      close();
    } else if (e.key === 'Enter' && !modal.hidden) {
      const first = results.querySelector('a');
      if (first) first.click();
    }
  });

  // A visible affordance too — ⌘K is invisible to anyone who does not already
  // know it, and on a phone there is no keyboard to press it on.
  for (const trigger of document.querySelectorAll('[data-docsearch]')) {
    trigger.addEventListener('click', (e) => {
      e.preventDefault();
      open();
    });
  }
})();
