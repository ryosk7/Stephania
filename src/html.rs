pub const INDEX: &str = r#"<!doctype html>
<html lang="ja"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>T3Box</title><style>
:root{font-family:system-ui,sans-serif;color-scheme:dark;background:#111827;color:#f3f4f6}
body{max-width:760px;margin:2rem auto;padding:0 1rem}section{border:1px solid #374151;border-radius:12px;padding:1.2rem;margin:1rem 0;background:#1f2937}
input,button{font:inherit;padding:.7rem;border-radius:8px;border:1px solid #64748b}
button{background:#2563eb;color:white;cursor:pointer;margin:.3rem}
input{background:#111827;color:white;max-width:90%}label{display:block;margin:.6rem 0 .3rem}
pre{white-space:pre-wrap;overflow-wrap:anywhere;font-size:.85rem}small{color:#cbd5e1}</style></head><body>
<h1>T3Box</h1>
<section><h2>配信状態</h2><button id="start">配信開始</button><button id="stop">停止</button>
<pre id="status">接続中...</pre></section>
<section><h2>Wi-Fi (会場)</h2><button id="scan">SSID検索</button><pre id="networks"></pre>
<label>SSID <input id="ssid" maxlength="32" autocomplete="off"></label>
<label>パスワード <input id="password" type="password" autocomplete="off"></label>
<button id="connect">Wi-Fiに接続</button><small>WPA-PSKのみ。ゲストログインや企業認証には非対応。</small></section>
<p role="status" id="message"></p>
<script>
"use strict";
async function api(path,method="GET",body) {
 const response=await fetch(path,{method,headers:body?{"Content-Type":"application/x-www-form-urlencoded"}:{},body,cache:"no-store"});
 const data=await response.json();
 if(!response.ok)throw new Error(data.error||response.status);
 return data;
}
function msg(s){document.getElementById("message").textContent=s}
async function refresh(){try{document.getElementById("status").textContent=JSON.stringify(await api("/status"),null,2)}catch(e){msg(String(e))}}
document.getElementById("start").onclick=async()=>{try{msg(JSON.stringify(await api("/stream/start","POST")));await refresh()}catch(e){msg(String(e))}};
document.getElementById("stop").onclick=async()=>{try{msg(JSON.stringify(await api("/stream/stop","POST")));await refresh()}catch(e){msg(String(e))}};
document.getElementById("scan").onclick=async()=>{try{document.getElementById("networks").textContent=(await api("/wifi/networks")).networks.join("\n")}catch(e){msg(String(e))}};
document.getElementById("connect").onclick=async()=>{
 const body=new URLSearchParams({ssid:document.getElementById("ssid").value,password:document.getElementById("password").value});
 document.getElementById("password").value="";
 try{msg(JSON.stringify(await api("/wifi/connect","POST",body)))}catch(e){msg(String(e))}
};
refresh();setInterval(refresh,5000);
</script></body></html>"#;
