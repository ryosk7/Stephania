// Point the download buttons at the latest published release's image ZIP.
// Without a published release they keep linking to the releases page.
(async () => {
  const asset = 'stephania-os-pi5.zip';
  try {
    const r = await fetch('https://api.github.com/repos/ryosk7/Stephania/releases/latest', {headers: {Accept: 'application/vnd.github+json'}});
    if (!r.ok) throw new Error(String(r.status));
    const release = await r.json();
    const zip = (release.assets || []).find(a => a.name === asset);
    if (!zip) throw new Error('no asset');
    const mb = Math.round(zip.size / 1e6);
    for (const a of document.querySelectorAll('#download, [data-download]')) a.href = zip.browser_download_url;
    document.getElementById('download-label').textContent = `Download ${release.tag_name} for Raspberry Pi 5`;
    document.getElementById('download-meta').textContent = `${asset} · ${mb} MB · signed · extract onto a microSD card, no flashing tool`;
  } catch {
    document.getElementById('download-meta').textContent = 'No published release yet: the button opens the releases page.';
  }
})();
