# Наскрізна перевірка сервера ЛІЇ: реєстрація, Yggdrasil (з перевіркою підпису через openssl), скіни,
# друзі, статуси (WebSocket), ретранслятор, збірки.
# Локально: запустити lia-server з LIA_HTTP=127.0.0.1:18080 LIA_PLAY_DOMAIN=play.localhost, потім `python e2e.py`.
# Бойовий: LIA_B=https://lia.starnure.dev SKIP_RELAY=1 python e2e.py (створює тестові акаунти — приберіть після).
# Ліміт реєстрацій — 5 на годину з IP: між прогонами перезапускайте сервер.
import base64, hashlib, io, json, os, socket, struct, subprocess, sys, threading, time, urllib.request, zipfile

B = os.environ.get("LIA_B", "http://127.0.0.1:18080")
RELAY = (os.environ.get("LIA_RELAY_HOST", "127.0.0.1"), 25570)
PLAY = os.environ.get("LIA_PLAY", "play.localhost")
HERE = __import__("tempfile").mkdtemp(prefix="lia-e2e-")  # pub.pem, підпис — для перевірки openssl
run = str(int(time.time()))[-5:]


def req(method, path, body=None, token=None, raw=None, ctype="application/json"):
    data = raw if raw is not None else (json.dumps(body).encode() if body is not None else None)
    r = urllib.request.Request(B + path, data=data, method=method, headers={"User-Agent": "LIA-Launcher/e2e"})
    if data is not None:
        r.add_header("Content-Type", ctype)
    if token:
        r.add_header("Authorization", "Bearer " + token)
    try:
        with urllib.request.urlopen(r) as resp:
            b = resp.read()
            return resp.status, (json.loads(b) if b and resp.headers.get("content-type", "").startswith("application/json") else b)
    except urllib.error.HTTPError as e:
        b = e.read()
        return e.code, json.loads(b) if b else None


a, b = "Alice" + run, "Bob" + run
s, ra = req("POST", "/api/register", {"name": a, "password": "password1"})
assert s == 200, (s, ra)
s, rb = req("POST", "/api/register", {"name": b, "password": "password2"})
assert s == 200, rb
assert req("POST", "/api/register", {"name": a.lower(), "password": "password1"})[0] == 409
assert req("POST", "/api/register", {"name": "x", "password": "password1"})[0] == 400
ta, tb = ra["accessToken"], rb["accessToken"]

# Yggdrasil
s, meta = req("GET", "/yggdrasil/")
assert meta["meta"]["feature.non_email_login"] and "BEGIN PUBLIC KEY" in meta["signaturePublickey"]
s, auth = req("POST", "/yggdrasil/authserver/authenticate", {"username": a, "password": "password1", "clientToken": "c1", "requestUser": True})
assert s == 200 and auth["selectedProfile"]["name"] == a and auth["clientToken"] == "c1", auth
assert req("POST", "/yggdrasil/authserver/authenticate", {"username": a, "password": "bad"})[0] == 403
assert req("POST", "/yggdrasil/authserver/validate", {"accessToken": auth["accessToken"]})[0] == 204
s, ref = req("POST", "/yggdrasil/authserver/refresh", {"accessToken": auth["accessToken"], "clientToken": "c1"})
assert s == 200 and ref["accessToken"] != auth["accessToken"]
assert req("POST", "/yggdrasil/authserver/validate", {"accessToken": auth["accessToken"]})[0] == 403
pid = ref["selectedProfile"]["id"]

# скін
def make_png(w, h):
    import zlib
    raw = b"".join(b"\x00" + bytes([200, 140, 98, 255]) * w for _ in range(h))
    ch = lambda t, d: struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)
    return b"\x89PNG\r\n\x1a\n" + ch(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0)) + ch(b"IDAT", zlib.compress(raw)) + ch(b"IEND", b"")


png = make_png(64, 64)
s, me = req("PUT", "/api/me/skin?slim=true", raw=png, token=ta, ctype="image/png")
assert s == 200 and me["slim"] and me["skin"].startswith(B + "/textures/"), me
s, tex = req("GET", me["skin"][len(B):])
assert s == 200 and tex == png

# join / hasJoined з перевіркою підпису
assert req("POST", "/yggdrasil/sessionserver/session/minecraft/join", {"accessToken": ref["accessToken"], "selectedProfile": pid, "serverId": "srv1"})[0] == 204
s, prof = req("GET", f"/yggdrasil/sessionserver/session/minecraft/hasJoined?username={a}&serverId=srv1")
assert s == 200 and prof["id"] == pid, (s, prof)
p = prof["properties"][0]
val = json.loads(base64.b64decode(p["value"]))
assert val["textures"]["SKIN"]["metadata"]["model"] == "slim"
open(os.path.join(HERE, "pub.pem"), "w").write(meta["signaturePublickey"])
open(os.path.join(HERE, "val.bin"), "wb").write(p["value"].encode())
open(os.path.join(HERE, "sig.bin"), "wb").write(base64.b64decode(p["signature"]))
v = subprocess.run(["openssl", "dgst", "-sha1", "-verify", "pub.pem", "-signature", "sig.bin", "val.bin"], cwd=HERE, capture_output=True, text=True)
assert "Verified OK" in v.stdout, v
assert req("GET", f"/yggdrasil/sessionserver/session/minecraft/hasJoined?username={a}&serverId=srv1")[0] == 204  # одноразово
s, prof2 = req("GET", f"/yggdrasil/sessionserver/session/minecraft/profile/{pid}")
assert s == 200 and "signature" not in prof2["properties"][0]
s, names = req("POST", "/yggdrasil/api/profiles/minecraft", [a, b, "nobody"])
assert len(names) == 2

# друзі
assert req("POST", f"/api/friends/{b}", token=ta)[0] == 204
s, fb = req("GET", "/api/friends", token=tb)
assert [x["name"] for x in fb["incoming"]] == [a], fb
s, found = req("GET", f"/api/users?q={b[:4]}", token=ta)
assert any(x["name"] == b for x in found)


# WebSocket: Боб бачить статус Аліси
def ws_connect(token):
    import ssl
    if B.startswith("https"):
        host = B.split("//")[1]
        sock = ssl.create_default_context().wrap_socket(socket.create_connection((host, 443)), server_hostname=host)
    else:
        sock = socket.create_connection(("127.0.0.1", 18080))
    key = base64.b64encode(os.urandom(16)).decode()
    sock.sendall(f"GET /api/ws?token={token} HTTP/1.1\r\nHost: {B.split('//')[1]}\r\nUser-Agent: Mozilla/5.0 LIA\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n".encode())
    resp = b""
    while b"\r\n\r\n" not in resp:
        resp += sock.recv(1)
    assert b"101" in resp.split(b"\r\n")[0], resp
    return sock


def ws_send(sock, text):
    d = text.encode()
    mask = os.urandom(4)
    hdr = bytes([0x81]) + (bytes([0x80 | len(d)]) if len(d) < 126 else bytes([0x80 | 126]) + struct.pack(">H", len(d)))
    sock.sendall(hdr + mask + bytes(c ^ mask[i % 4] for i, c in enumerate(d)))


def ws_recv(sock, timeout=3):
    sock.settimeout(timeout)
    while True:
        h = sock.recv(2)
        n = h[1] & 0x7F
        if n == 126:
            n = struct.unpack(">H", sock.recv(2))[0]
        data = b""
        while len(data) < n:
            data += sock.recv(n - len(data))
        if h[0] & 0x0F == 1:
            return json.loads(data)


wb = ws_connect(tb)
assert req("POST", f"/api/friends/{a}", token=tb)[0] == 204  # прийняв
s, fa = req("GET", "/api/friends", token=ta)
assert [x["name"] for x in fa["friends"]] == [b], fa
wa = ws_connect(ta)
ws_send(wa, json.dumps({"type": "presence", "state": "playing", "game": {"name": "Тест", "mcVersion": "1.21.1", "loader": "fabric"}, "world": {"code": "zzzzzz"}}))
m = ws_recv(wb)
assert m["type"] == "presence" and m["name"] == a and m["presence"]["game"]["mcVersion"] == "1.21.1" and "world" not in m["presence"], m

if os.environ.get("SKIP_RELAY"):
    print("e2e (без ретранслятора): ok")
    sys.exit(0)
# ретранслятор: хост → код → гравець через handshake
lan = socket.socket()
lan.bind(("127.0.0.1", 0))
lan.listen()
lan_port = lan.getsockname()[1]
ctl = socket.create_connection(RELAY)
ctl.sendall(b"\x00LIA" + json.dumps({"kind": "host", "token": ta}).encode() + b"\n")
f = ctl.makefile("r")
hello = json.loads(f.readline())
code = hello["code"]
assert hello["address"] == f"{code}.{PLAY}:25570", hello
ws_send(wa, json.dumps({"type": "presence", "state": "playing", "game": {}, "world": {"code": code}}))
m = ws_recv(wb)
assert m["presence"]["world"]["address"] == hello["address"], m


def host_side():
    for line in f:
        msg = json.loads(line)
        if "open" in msg:
            d = socket.create_connection(RELAY)
            d.sendall(b"\x00LIA" + json.dumps({"kind": "data", "id": msg["open"]}).encode() + b"\n")
            c = socket.create_connection(("127.0.0.1", lan_port))
            def pipe(x, y):
                try:
                    while (chunk := x.recv(4096)):
                        y.sendall(chunk)
                except OSError:
                    pass
            threading.Thread(target=pipe, args=(d, c), daemon=True).start()
            threading.Thread(target=pipe, args=(c, d), daemon=True).start()
            return


threading.Thread(target=host_side, daemon=True).start()


def lan_server():
    srv, _ = lan.accept()
    got = b""
    while not got.endswith(b"hello-from-player"):
        got += srv.recv(4096)
    assert PLAY.encode() in got
    srv.sendall(b"pong-from-lan")


threading.Thread(target=lan_server, daemon=True).start()


def varint(n):
    out = b""
    while True:
        b7 = n & 0x7F
        n >>= 7
        out += bytes([b7 | (0x80 if n else 0)])
        if not n:
            return out


host = (code + "." + PLAY).encode() + b"\x00FML3\x00"
pkt = varint(0) + varint(767) + varint(len(host)) + host + struct.pack(">H", 25570) + varint(2)
pl = socket.create_connection(RELAY)
pl.sendall(varint(len(pkt)) + pkt + b"hello-from-player")
pl.settimeout(5)
assert pl.recv(100) == b"pong-from-lan"

# невідомий код → повідомлення про закритий світ
host2 = f"nonexx.{PLAY}".encode()
pkt2 = varint(0) + varint(767) + varint(len(host2)) + host2 + struct.pack(">H", 25570) + varint(2)
p2 = socket.create_connection(RELAY)
p2.sendall(varint(len(pkt2)) + pkt2)
p2.settimeout(3)
assert "закрито".encode() in p2.recv(1024)

f.close()
ctl.close()
m = ws_recv(wb)
assert "world" not in m["presence"], m

# збірки
buf = io.BytesIO()
with zipfile.ZipFile(buf, "w") as z:
    z.writestr("modrinth.index.json", json.dumps({"formatVersion": 1, "game": "minecraft", "versionId": "1", "name": "Тестова збірка", "files": [], "dependencies": {"minecraft": "1.21.1", "fabric-loader": "0.16.0"}}))
s, pack = req("POST", "/api/packs?visibility=friends&description=%D0%BE%D0%BF%D0%B8%D1%81", raw=buf.getvalue(), token=ta, ctype="application/zip")
assert s == 200 and pack["loader"] == "fabric" and pack["mcVersion"] == "1.21.1" and pack["version"] == 1, pack
s, lb = req("GET", "/api/packs", token=tb)
assert [x["id"] for x in lb] == [pack["id"]]
assert req("POST", f"/api/packs/{pack['id']}/subscribe", token=tb)[0] == 204
s, upd = req("PUT", f"/api/packs/{pack['id']}", raw=buf.getvalue(), token=ta, ctype="application/zip")
assert upd["version"] == 2
seen = set()
for _ in range(3):
    seen.add(ws_recv(wb)["type"])
assert "pack" in seen, seen
s, subs = req("GET", "/api/packs/subscribed", token=tb)
assert subs[0]["version"] == 2 and subs[0]["subscribed"]
s, file = req("GET", f"/api/packs/{pack['id']}/file", token=tb)
assert s == 200 and file == buf.getvalue()
s, feed = req("GET", "/api/feed", token=tb)
assert {"pack_updated", "pack_published", "friend"} <= {e["kind"] for e in feed}, feed

# незнайомцю приватна збірка не видна
s, rc = req("POST", "/api/register", {"name": "Carol" + run, "password": "password3"})
assert req("GET", f"/api/packs/{pack['id']}/file", token=rc["accessToken"])[0] == 404
print("e2e: ok")
