(function () {
  if (typeof document === 'undefined') return;
  var current = '';
  var $ = function (id) { return document.getElementById(id); };
  async function generate() {
    current = await T3BoxSetup.uriFor(T3BoxSetup.newSecret());
    $('qr').innerHTML = T3BoxSetup.svg(T3BoxSetup.matrix(current));
    var blob = new Blob([current + '\n'], {type: 'text/plain'});
    var a = $('download');
    if (a.href) URL.revokeObjectURL(a.href);
    a.href = URL.createObjectURL(blob);
    a.download = 't3box-setup.txt';
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
