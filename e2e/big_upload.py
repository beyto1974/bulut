"""Usage: python3 e2e/big_upload.py http://localhost:<port>   (needs a running app)

End-to-end check: 1 GB chunked upload with a simulated resume, then verified download."""
import hashlib, json, os, sys, time, urllib.request

BASE = sys.argv[1]
SIZE = 1_000_000_000
PATH = os.path.join(os.environ.get("TMPDIR", "/tmp"), "bulut-big.bin")


def call(method, path, body=None, headers=None):
    req = urllib.request.Request(BASE + path, data=body, method=method, headers=headers or {})
    with urllib.request.urlopen(req) as r:
        data = r.read()
        return r.status, dict(r.headers), data


def jcall(method, path, obj=None):
    body = json.dumps(obj).encode() if obj is not None else None
    s, h, d = call(method, path, body, {"content-type": "application/json"} if body else {})
    return s, (json.loads(d) if d else None)


if not os.path.exists(PATH) or os.path.getsize(PATH) != SIZE:
    with open(PATH, "wb") as f, open("/dev/urandom", "rb") as r:
        left = SIZE
        while left:
            n = min(left, 16 * 1024 * 1024)
            f.write(r.read(n))
            left -= n
want = hashlib.sha256()
with open(PATH, "rb") as f:
    for chunk in iter(lambda: f.read(8 * 1024 * 1024), b""):
        want.update(chunk)
print("source sha256", want.hexdigest()[:16])

_, session = jcall("POST", "/api/s", {"description": "1 GB e2e"})
code = session["code"]
_, up = jcall("POST", f"/api/s/{code}/uploads", {"name": "big.bin", "size": SIZE, "content_type": "application/octet-stream"})
uid, psize, total = up["upload_id"], up["part_size"], up["parts_total"]
print("session", code, "parts", total, "part_size", psize)


def send(f, n):
    f.seek((n - 1) * psize)
    s, _, _ = call("PUT", f"/api/s/{code}/uploads/{uid}/parts/{n}", f.read(psize))
    assert s == 204, s


t0 = time.time()
with open(PATH, "rb") as f:
    for n in range(1, total // 2 + 1):  # first half, then "lose the connection"
        send(f, n)
print("first half sent in %.1fs" % (time.time() - t0))

_, st = jcall("GET", f"/api/s/{code}/uploads/{uid}")
have = set(st["parts_received"])
print("resume: server has", len(have), "of", total)
assert have == set(range(1, total // 2 + 1))
try:
    jcall("POST", f"/api/s/{code}/uploads/{uid}/complete")
    raise SystemExit("complete should have failed with parts missing")
except urllib.error.HTTPError as e:
    assert e.code == 409, e.code

with open(PATH, "rb") as f:
    for n in range(1, total + 1):
        if n not in have:
            send(f, n)
s, stored = jcall("POST", f"/api/s/{code}/uploads/{uid}/complete")
assert s == 201 and stored["version"]["size"] == SIZE, stored
print("complete ok in %.1fs total" % (time.time() - t0))

vid = stored["version"]["id"]
got = hashlib.sha256()
t1 = time.time()
req = urllib.request.Request(BASE + f"/api/s/{code}/versions/{vid}/download")
with urllib.request.urlopen(req) as r:
    assert int(r.headers["content-length"]) == SIZE
    while True:
        b = r.read(8 * 1024 * 1024)
        if not b:
            break
        got.update(b)
print("downloaded in %.1fs, sha256 %s" % (time.time() - t1, got.hexdigest()[:16]))
assert got.hexdigest() == want.hexdigest(), "HASH MISMATCH"

# Range from the middle.
off = 123_456_789
s, h, d = call("GET", f"/api/s/{code}/versions/{vid}/download", headers={"Range": f"bytes={off}-{off + 99}"})
with open(PATH, "rb") as f:
    f.seek(off)
    assert s == 206 and d == f.read(100), "range mismatch"
print("range ok; content-range", h.get("Content-Range") or h.get("content-range"))

s, _, _ = call("DELETE", f"/api/s/{code}")
os.remove(PATH)
print("session deleted", s, "\nPASS")
