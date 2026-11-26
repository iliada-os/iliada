// Run in the page by Homer's Chrome bridge: the page's readable text and
// its controls, each marked with an id Homer can click or fill.
(() => {
  const MAX_TEXT = 8000, MAX_ELEMENTS = 250;
  const sel = 'a[href], button, input, select, textarea, summary, [contenteditable="true"], [contenteditable=""], ' +
    '[role=button], [role=link], [role=checkbox], [role=radio], [role=tab], [role=menuitem], [role=option], ' +
    '[role=textbox], [role=combobox], [role=switch], [role=searchbox], [tabindex]:not([tabindex="-1"])';
  const visible = (el) => {
    const r = el.getBoundingClientRect();
    if (r.width < 1 || r.height < 1) return false;
    const s = getComputedStyle(el);
    return s.visibility !== 'hidden' && s.display !== 'none' && s.opacity !== '0';
  };
  const clean = (s, n) => (s || '').replace(/\s+/g, ' ').trim().slice(0, n);
  const label = (el) => {
    let t = el.getAttribute('aria-label') || '';
    if (!t && el.labels && el.labels.length) t = el.labels[0].innerText;
    if (!t && el.getAttribute('aria-labelledby')) {
      const l = document.getElementById(el.getAttribute('aria-labelledby'));
      if (l) t = l.innerText;
    }
    if (!t && el.tagName !== 'INPUT' && el.tagName !== 'TEXTAREA' && el.tagName !== 'SELECT') t = el.innerText;
    if (!t && el.type === 'submit') t = el.value;
    if (!t) t = el.getAttribute('placeholder') || el.getAttribute('title') || el.getAttribute('alt') || el.getAttribute('name') || '';
    return clean(t, 120);
  };
  const docs = [document];
  for (const f of document.querySelectorAll('iframe')) {
    try { if (f.contentDocument && visible(f)) docs.push(f.contentDocument); } catch (e) {}
  }
  let next = Number(document.documentElement.dataset.homerNext || '1');
  const elements = [];
  for (const d of docs) {
    for (const el of d.querySelectorAll(sel)) {
      if (elements.length >= MAX_ELEMENTS) break;
      if (!visible(el) || el.disabled) continue;
      if (!el.dataset.homerId) el.dataset.homerId = String(next++);
      const tag = el.tagName.toLowerCase();
      const e = { id: el.dataset.homerId, tag, role: el.getAttribute('role') || '', label: label(el) };
      if (tag === 'input') e.type = el.type;
      if (tag === 'a') e.href = clean(el.href, 200);
      if ((tag === 'input' || tag === 'textarea' || tag === 'select') && el.type !== 'password' && el.type !== 'hidden')
        e.value = clean(el.value, 200);
      if (el.isContentEditable) e.editable = true;
      if (el.form) e.in_form = (el.form.method || 'get').toLowerCase();
      if (el === document.activeElement) e.focused = true;
      elements.push(e);
    }
  }
  document.documentElement.dataset.homerNext = String(next);
  const text = clean(document.body ? document.body.innerText : '', MAX_TEXT * 2);
  return { title: document.title, url: location.href, text: text.slice(0, MAX_TEXT), cut: text.length > MAX_TEXT, elements };
})()
