(function () {
  if (typeof document === 'undefined') return;
  var current = '';
  var $ = function (id) { return document.getElementById(id); };

  // Appearance: follow the system unless the visitor picked light or dark
  // (DESIGN.md §4). Stored in this browser only.
  var root = document.documentElement;
  function applyTheme(t) {
    root.classList.toggle('light', t === 'light');
    root.classList.toggle('dark', t === 'dark');
    var buttons = document.querySelectorAll('[data-theme]');
    for (var i = 0; i < buttons.length; i++) buttons[i].setAttribute('aria-checked', String(buttons[i].getAttribute('data-theme') === t));
  }
  var saved = 'system';
  try { saved = localStorage.getItem('stephania.appearance') || 'system'; } catch (e) { /* storage disabled */ }
  if (saved !== 'light' && saved !== 'dark') saved = 'system';
  applyTheme(saved);
  document.querySelector('.theme').addEventListener('click', function (e) {
    var b = e.target.closest('[data-theme]');
    if (!b) return;
    var t = b.getAttribute('data-theme');
    applyTheme(t);
    try { localStorage.setItem('stephania.appearance', t); } catch (err) { /* this page only */ }
  });

  async function generate() {
    current = await StephaniaSetup.uriFor(StephaniaSetup.newSecret());
    $('qr').innerHTML = StephaniaSetup.svg(StephaniaSetup.matrix(current));
    var blob = new Blob([current + '\n'], {type: 'text/plain'});
    var a = $('download');
    if (a.href && a.href.indexOf('blob:') === 0) URL.revokeObjectURL(a.href);
    a.href = URL.createObjectURL(blob);
    a.download = 'stephania-setup.txt';
    $('steps').hidden = false;
  }
  $('generate').addEventListener('click', function () {
    generate().catch(function () { $('error').hidden = false; });
  });
  $('clear').addEventListener('click', function () {
    current = '';
    $('qr').innerHTML = '';
    $('steps').hidden = true;
  });
  if (!window.crypto || !crypto.subtle) $('error').hidden = false;
})();
