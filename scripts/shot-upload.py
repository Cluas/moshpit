#!/usr/bin/env python3
# Tiny phone→Mac screenshot upload page. Open the URL on the phone (same Wi-Fi),
# pick a screenshot, tap 上传 — it lands in DEST so Claude can read it directly.
import http.server, os, time, urllib.parse
DEST = os.path.expanduser("~/Downloads/beacon-shots")
os.makedirs(DEST, exist_ok=True)
PAGE = ("""<!doctype html><meta name=viewport content="width=device-width,initial-scale=1">
<title>Beacon → Claude</title>
<style>body{font:17px -apple-system,system-ui;background:#0b0b0d;color:#eee;margin:0;
padding:32px 24px;text-align:center}h2{color:#53dcc9}
.b{background:#53dcc9;color:#001018;border:0;border-radius:14px;padding:16px 26px;
font-size:18px;font-weight:700;margin-top:20px}input{font-size:16px;color:#eee;margin-top:10px}
#s{margin-top:18px;color:#9fe}</style>
<h2>Beacon → Claude</h2><p>选一张截图,点上传</p>
<input type=file accept=image/* id=f><br><button class=b onclick=up()>上传</button>
<p id=s></p><script>
async function up(){let f=document.getElementById('f').files[0];if(!f)return;
document.getElementById('s').textContent='上传中…';
let r=await fetch('/u?n='+encodeURIComponent(f.name||'shot.png'),{method:'POST',body:f});
document.getElementById('s').textContent=r.ok?'\\u2713 已上传,去告诉 Claude':'\\u2717 失败,重试';}
</script>""").encode("utf-8")

class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(PAGE)))
        self.end_headers(); self.wfile.write(PAGE)
    def do_POST(self):
        q = urllib.parse.parse_qs(urllib.parse.urlparse(self.path).query)
        name = os.path.basename(q.get("n", ["shot.png"])[0]) or "shot.png"
        n = int(self.headers.get("Content-Length", 0))
        data = self.rfile.read(n)
        path = os.path.join(DEST, f"{time.strftime('%H%M%S')}_{name}")
        with open(path, "wb") as fp: fp.write(data)
        print("saved", path, len(data), "bytes", flush=True)
        self.send_response(200); self.end_headers(); self.wfile.write(b"ok")
    def log_message(self, *a): pass

print("upload server on :8788 -> ", DEST, flush=True)
http.server.HTTPServer(("0.0.0.0", 8788), H).serve_forever()
