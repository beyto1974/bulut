"""Usage: python3 e2e/parallel_upload.py http://localhost:<port>   (needs a running app)

One 1 GB file is uploaded while 20 small files are uploaded next to it. Every upload is independent,
so the small files must finish long before the big one and nothing may be lost or mixed up.
"""
import concurrent.futures as cf
import hashlib
import json
import os
import sys
import threading
import time
import urllib.request

BASE = sys.argv[1]
BIG = int(os.environ.get("BULUT_E2E_BIG", 1_000_000_000))
SMALL_COUNT = 20
SMALL_SIZE = 100_000


def call(method, path, body=None, headers=None):
    req = urllib.request.Request(BASE + path, data=body, method=method, headers=headers or {})
    with urllib.request.urlopen(req) as r:
        data = r.read()
        return r.status, (json.loads(data) if data and r.headers.get("content-type", "").startswith("application/json") else data)


def jcall(method, path, obj=None):
    body = json.dumps(obj).encode() if obj is not None else None
    return call(method, path, body, {"content-type": "application/json"} if body else {})


def upload(code, name, payload_for_part, size):
    """Chunked upload; payload_for_part(n, length) returns the bytes of part n."""
    _, up = jcall("POST", f"/api/s/{code}/uploads", {"name": name, "size": size})
    uid, psize, total = up["upload_id"], up["part_size"], up["parts_total"]
    for n in range(1, total + 1):
        length = min(psize, size - (n - 1) * psize)
        s, _ = call("PUT", f"/api/s/{code}/uploads/{uid}/parts/{n}", payload_for_part(n, length))
        assert s == 204
    s, stored = jcall("POST", f"/api/s/{code}/uploads/{uid}/complete")
    assert s == 201
    return stored


_, session = jcall("POST", "/api/s", {"description": "parallel upload test"})
code = session["code"]
start = time.time()
done_at = {}
lock = threading.Lock()


def small(i):
    data = (f"small file {i} ".encode() * 10000)[:SMALL_SIZE]
    stored = upload(code, f"small-{i:02d}.txt", lambda n, l: data, len(data))
    with lock:
        done_at[f"small-{i:02d}"] = time.time() - start
    return hashlib.sha256(data).hexdigest(), stored["version"]["size"]


def big():
    zero = bytes(8 * 1024 * 1024)
    stored = upload(code, "big.bin", lambda n, l: zero[:l], BIG)
    with lock:
        done_at["big"] = time.time() - start
    return stored["version"]["size"]


with cf.ThreadPoolExecutor(max_workers=6) as pool:
    big_future = pool.submit(big)
    time.sleep(0.3)  # the big upload is under way before the small ones start
    small_futures = [pool.submit(small, i) for i in range(SMALL_COUNT)]
    smalls = [f.result() for f in small_futures]
    big_size = big_future.result()

last_small = max(t for k, t in done_at.items() if k != "big")
print("small files: %d done, last one after %.1fs" % (SMALL_COUNT, last_small))
print("big file: %d bytes, done after %.1fs" % (big_size, done_at["big"]))
assert big_size == BIG
assert all(size == SMALL_SIZE for _, size in smalls)
assert last_small < done_at["big"], "small files must not wait for the big one"

# Everything is there, once each, and the small files have the right content.
_, listing = jcall("GET", f"/api/s/{code}/files")
names = sorted(i["name"] for i in listing["items"])
assert names == sorted(["big.bin"] + [f"small-{i:02d}.txt" for i in range(SMALL_COUNT)]), names
assert all(i["version_count"] == 1 for i in listing["items"])
for i in (0, SMALL_COUNT - 1):
    _, body = call("GET", f"/api/s/{code}/download?name=small-{i:02d}.txt")
    assert hashlib.sha256(body).hexdigest() == smalls[i][0], f"small-{i:02d} content"

# Two uploads of the same name at the same time end as two versions, not as a failure.
with cf.ThreadPoolExecutor(max_workers=2) as pool:
    same = list(pool.map(lambda k: upload(code, "same.txt", lambda n, l: b"x" * l, 50)["version"]["version"], range(2)))
assert sorted(same) == [1, 2], same
print("same name in parallel: versions", sorted(same))

call("DELETE", f"/api/s/{code}")
print("PASS")
