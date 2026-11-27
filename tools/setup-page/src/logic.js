// T3Box setup secret generation. Runs entirely in the browser; nothing is
// sent anywhere (the page's CSP forbids all network access).
// Format (contracts/fixtures/ble/setup-uris.json):
//   t3box-setup:v1:<32 hex secret>:<first 8 hex of SHA-256(secret)>
var T3BoxSetup = (function () {
  function hex(bytes) {
    return Array.prototype.map.call(bytes, function (b) { return (b < 16 ? '0' : '') + b.toString(16); }).join('');
  }
  async function uriFor(secret) {
    var digest = new Uint8Array(await crypto.subtle.digest('SHA-256', secret));
    return 't3box-setup:v1:' + hex(secret) + ':' + hex(digest.subarray(0, 4));
  }
  function newSecret() {
    var s = new Uint8Array(16);
    do { crypto.getRandomValues(s); } while (s.every(function (b) { return b === 0; }));
    return s;
  }
  /** QR as a boolean matrix (error correction M, auto version). */
  function matrix(text) {
    var qr = qrcode(0, 'M');
    qr.addData(text, 'Byte');
    qr.make();
    var n = qr.getModuleCount(), rows = [];
    for (var r = 0; r < n; r++) { var row = []; for (var c = 0; c < n; c++) row.push(qr.isDark(r, c)); rows.push(row); }
    return rows;
  }
  function svg(rows) {
    var n = rows.length, q = 4, path = '';
    rows.forEach(function (row, r) { row.forEach(function (d, c) { if (d) path += 'M' + (c + q) + ' ' + (r + q) + 'h1v1h-1z'; }); });
    return '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ' + (n + 2 * q) + ' ' + (n + 2 * q) +
      '" shape-rendering="crispEdges" role="img" aria-label="T3Box setup QR code"><rect width="100%" height="100%" fill="#fff"/><path d="' + path + '" fill="#000"/></svg>';
  }
  return {hex: hex, uriFor: uriFor, newSecret: newSecret, matrix: matrix, svg: svg};
})();
