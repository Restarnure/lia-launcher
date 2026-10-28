# Наскрізна перевірка сервера ЛІЇ: акаунт (пошта, вхід, пристрої, паролі), Yggdrasil (з перевіркою підпису
# через openssl), друзі, статуси (WebSocket), ретранслятор, збірки.
# Локально: запустити lia-server з LIA_HTTP=127.0.0.1:18080 LIA_PLAY_DOMAIN=play.localhost LIA_MAIL_LOG=1,
# вивід сервера — у файл, і `LIA_SERVER_LOG=<файл> python e2e.py` (коди з листів читаються з логу).
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


LOG = os.environ.get("LIA_SERVER_LOG")


def mail_code(email, after=0):
    """Останній код, «надісланий» на email (LIA_MAIL_LOG=1 друкує листи в лог сервера)."""
    import re
    for _ in range(20):
        lines = [l for l in open(LOG, encoding="utf-8", errors="replace").read().splitlines()[after:] if f"MAIL to={email} " in l]
        codes = [m.group(1) for l in lines for m in [re.search(r"Код: (\d{6})", l)] if m]
        if codes:
            return codes[-1]
        time.sleep(0.2)
    raise AssertionError(f"нема листа з кодом для {email}")


def log_len():
    return len(open(LOG, encoding="utf-8", errors="replace").read().splitlines()) if LOG else 0


pw = lambda n: f"zelenyi-kit-{n}-{run}"
a, b = "Alice" + run, "Bob" + run
ea, eb = f"alice{run}@test.lia", f"bob{run}@test.lia"
s, ra = req("POST", "/api/register", {"name": a, "email": ea, "password": pw(1), "device": "ПК Аліси"})
assert s == 200, (s, ra)
s, rb = req("POST", "/api/register", {"name": b, "email": eb, "password": pw(2), "device": "ПК Боба"})
assert s == 200, rb
assert req("POST", "/api/register", {"name": a.lower(), "email": "x" + ea, "password": pw(1)})[0] == 409  # нік
assert req("POST", "/api/register", {"name": "Other" + run, "email": ea.upper(), "password": pw(1)})[0] == 409  # пошта
assert req("POST", "/api/register", {"name": "x", "email": "x@x.ua", "password": pw(1)})[0] == 400
assert req("POST", "/api/register", {"name": "Weak" + run, "email": f"w{run}@test.lia", "password": "password1"})[0] == 400  # простий пароль
assert req("POST", "/api/register", {"name": "admin", "email": f"ad{run}@test.lia", "password": pw(9)})[0] == 400  # зарезервовано
ta, tb = ra["accessToken"], rb["accessToken"]

s, me0 = req("GET", "/api/me", token=ta)
assert me0["email"] == ea, me0
if me0["mailEnabled"]:
    assert not me0["emailVerified"] and LOG, "для перевірки пошти потрібен LIA_SERVER_LOG"
    s, e = req("GET", "/api/friends", token=ta)
    assert s == 403 and e["error"] == "EmailNotVerified", (s, e)  # без підтвердження соцмережа закрита
    assert req("POST", "/api/email/verify", {"code": "000000"}, token=ta)[0] == 400
    for who, tok, em in [(a, ta, ea), (b, tb, eb)]:
        s, v = req("POST", "/api/email/verify", {"code": mail_code(em)}, token=tok)
        assert s == 200 and v["emailVerified"], v
    assert req("GET", "/api/friends", token=ta)[0] == 200

# вхід у ЛІЯ ID (нік або пошта) і пристрої
assert req("POST", "/api/login", {"name": a, "password": "bad"})[0] == 403
s, login = req("POST", "/api/login", {"name": ea, "password": pw(1), "device": "Ноутбук"})
assert s == 200 and login["name"] == a, login
s, sess = req("GET", "/api/sessions", token=ta)
assert {x["device"] for x in sess} == {"ПК Аліси", "Ноутбук"} and sum(x["current"] for x in sess) == 1, sess
other = next(x for x in sess if not x["current"])
assert req("DELETE", f"/api/sessions/{other['id']}", token=ta)[0] == 204
assert req("GET", "/api/me", token=login["accessToken"])[0] == 401  # той пристрій вийшов

nick = "Nick" + run
s, game = req("POST", "/api/game/session", {"kind": "offline", "name": nick}, token=ta)
assert s == 200 and game["name"] == nick, game
assert req("POST", "/api/game/session", {"kind": "offline", "name": nick}, token=tb)[0] == 409  # нік чужий
assert req("GET", "/api/me", token=game["accessToken"])[0] == 401  # токен гри не відкриває API акаунта
s, me0 = req("GET", "/api/me", token=ta)
assert me0["profiles"] == [{"name": nick, "kind": "offline"}], me0
pid = game["uuid"]
s, meta = req("GET", "/yggdrasil/")
assert "BEGIN PUBLIC KEY" in meta["signaturePublickey"] and ".minecraft.net" in meta["skinDomains"], meta
assert req("POST", "/yggdrasil/authserver/validate", {"accessToken": game["accessToken"]})[0] == 204
assert req("POST", "/yggdrasil/authserver/validate", {"accessToken": ta})[0] == 403
# токен акаунта не годиться для входу в гру
assert req("POST", "/yggdrasil/sessionserver/session/minecraft/join", {"accessToken": ta, "selectedProfile": pid, "serverId": "x"})[0] == 403

# join / hasJoined з перевіркою підпису
assert req("POST", "/yggdrasil/sessionserver/session/minecraft/join", {"accessToken": game["accessToken"], "selectedProfile": pid, "serverId": "srv1"})[0] == 204
s, prof = req("GET", f"/yggdrasil/sessionserver/session/minecraft/hasJoined?username={nick}&serverId=srv1")
assert s == 200 and prof["id"] == pid, (s, prof)
p = prof["properties"][0]
val = json.loads(base64.b64decode(p["value"]))
assert val["profileName"] == nick and val["textures"] == {}, val  # офлайн-нік — стандартний скін
open(os.path.join(HERE, "pub.pem"), "w").write(meta["signaturePublickey"])
open(os.path.join(HERE, "val.bin"), "wb").write(p["value"].encode())
open(os.path.join(HERE, "sig.bin"), "wb").write(base64.b64decode(p["signature"]))
v = subprocess.run(["openssl", "dgst", "-sha1", "-verify", "pub.pem", "-signature", "sig.bin", "val.bin"], cwd=HERE, capture_output=True, text=True)
assert "Verified OK" in v.stdout, v
assert req("GET", f"/yggdrasil/sessionserver/session/minecraft/hasJoined?username={nick}&serverId=srv1")[0] == 204  # одноразово, далі — Mojang, який такого не знає
# ліцензійний гравець без ЛІЇ: профіль і скін з Mojang, перепідписані нашим ключем
for _ in range(3):  # сервер Mojang інколи відповідає порожньо — пробуємо ще
    s, notch = req("GET", "/yggdrasil/sessionserver/session/minecraft/profile/069a79f444e94726a5befca90e38aaf5?unsigned=false")
    if s == 200:
        break
    time.sleep(2)
assert s == 200 and notch["name"] == "Notch" and notch["properties"][0]["signature"], (s, notch)
s, prof2 = req("GET", f"/yggdrasil/sessionserver/session/minecraft/profile/{pid}")
assert s == 200 and "signature" not in prof2["properties"][0]
s, names = req("POST", "/yggdrasil/api/profiles/minecraft", [nick, b, "nobody"])
assert len(names) == 1, names

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
ec = f"carol{run}@test.lia"
s, rc = req("POST", "/api/register", {"name": "Carol" + run, "email": ec, "password": pw(3)})
tc = rc["accessToken"]
if me0["mailEnabled"]:
    req("POST", "/api/email/verify", {"code": mail_code(ec)}, token=tc)
assert req("GET", f"/api/packs/{pack['id']}/file", token=tc)[0] == 404

# відхилений запит у друзі: той, хто просив, не може засипати повторними
assert req("POST", f"/api/friends/{a}", token=tc)[0] == 204
assert [x["name"] for x in req("GET", "/api/friends", token=ta)[1]["incoming"]] == ["Carol" + run]
assert req("DELETE", f"/api/friends/Carol{run}", token=ta)[0] == 204  # Аліса відхиляє
assert req("GET", "/api/friends", token=ta)[1]["incoming"] == []
assert req("POST", f"/api/friends/{a}", token=tc)[0] == 204  # Керол пробує ще раз — тихо ігнорується
assert req("GET", "/api/friends", token=ta)[1]["incoming"] == []
assert [x["name"] for x in req("GET", "/api/friends", token=tc)[1]["outgoing"]] == [a]  # для Керол — «чекає»
# `_` у пошуку — звичайний символ, а не шаблон
assert req("GET", "/api/users?q=Alic_", token=tc)[1] == []

# зміна пароля: інші пристрої виходять, цей лишається
s, l2 = req("POST", "/api/login", {"name": "Carol" + run, "password": pw(3)})
assert req("POST", "/api/me/password", {"old": "bad", "new": pw(4)}, token=tc)[0] == 403
assert req("POST", "/api/me/password", {"old": pw(3), "new": pw(4)}, token=tc)[0] == 204
assert req("GET", "/api/me", token=l2["accessToken"])[0] == 401
assert req("GET", "/api/me", token=tc)[0] == 200

if me0["mailEnabled"]:
    # відновлення пароля: код з пошти, вихід з усіх пристроїв
    n = log_len()
    assert req("POST", "/api/password/forgot", {"login": ec}, token=None)[0] == 204
    assert req("POST", "/api/password/forgot", {"login": "nobody" + run}, token=None)[0] == 204  # не підказуємо, чи є акаунт
    code = mail_code(ec, n)
    assert req("POST", "/api/password/reset", {"login": ec, "code": "111111", "password": pw(5)})[0] == 400
    assert req("POST", "/api/password/reset", {"login": ec, "code": code, "password": pw(5)})[0] == 204
    assert req("GET", "/api/me", token=tc)[0] == 401
    s, l3 = req("POST", "/api/login", {"name": ec, "password": pw(5)})
    assert s == 200, l3
    tc = l3["accessToken"]
    # зміна пошти: код на нову адресу
    nc = f"carol2{run}@test.lia"
    assert req("POST", "/api/email/change", {"email": nc, "password": pw(5)}, token=tc)[0] == 204
    assert req("POST", "/api/email/verify", {"code": mail_code(nc)}, token=tc)[0] == 200
    assert req("GET", "/api/me", token=tc)[1]["email"] == nc
    pc = pw(5)
else:
    pc = pw(4)

# захист від підбору: після 8 невдалих спроб вхід закрито навіть з правильним паролем
for _ in range(8):
    assert req("POST", "/api/login", {"name": "Carol" + run, "password": "nope"})[0] == 403
s, e = req("POST", "/api/login", {"name": "Carol" + run, "password": pc})
assert s == 429, (s, e)

# видалення акаунта — лише з паролем
assert req("DELETE", "/api/me", {"password": "bad"}, token=tc)[0] == 403
assert req("DELETE", "/api/me", {"password": pc}, token=tc)[0] == 204
assert req("GET", "/api/me", token=tc)[0] == 401
print("e2e: ok")
