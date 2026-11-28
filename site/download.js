// Point the download buttons at the latest published release's image ZIP.
// Without a published release they keep linking to the releases page.
// The release key fingerprint is part of this site, not of the release, so
// a replaced release asset cannot replace it too (docs/RELEASE.md).
(async () => {
  const asset = 'stephania-os-pi5.zip';
  const releaseKeySha256 = 'ae3575735020476cc1a52c0dd8c7f0b8abf58ac57834f5c3714d4b1e232dc6a8';
  const verifyDocs = 'https://github.com/ryosk7/Stephania/blob/main/docs/RELEASE.md#verifying-a-download';
  const meta = document.getElementById('download-meta');
  const verify = document.createElement('p');
  verify.className = 'fine';
  verify.id = 'download-verify';
  verify.append('Check a download with ');
  const link = document.createElement('a');
  link.href = verifyDocs;
  link.textContent = 'os/release/verify.sh';
  verify.append(link, ` · release-signing.pub SHA-256 ${releaseKeySha256}`);
  meta.after(verify);
  try {
    const r = await fetch('https://api.github.com/repos/ryosk7/Stephania/releases/latest', {headers: {Accept: 'application/vnd.github+json'}});
    if (!r.ok) throw new Error(String(r.status));
    const release = await r.json();
    const zip = (release.assets || []).find(a => a.name === asset);
    if (!zip) throw new Error('no asset');
    const mb = Math.round(zip.size / 1e6);
    for (const a of document.querySelectorAll('#download, [data-download]')) a.href = zip.browser_download_url;
    document.getElementById('download-label').textContent = `Download ${release.tag_name} for Raspberry Pi 5`;
    meta.textContent = `${asset} · ${mb} MB · listed in the signed SHA256SUMS · extract onto a microSD card, no flashing tool`;
  } catch {
    meta.textContent = 'No published release yet: the button opens the releases page.';
  }
})();
