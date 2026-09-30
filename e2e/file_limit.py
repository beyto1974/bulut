"""Usage: python3 e2e/file_limit.py http://localhost:<port>   (needs a running app)

120 uploads of new names arrive at once in one session. Exactly MAX_FILES_PER_SESSION (100) are stored,
the rest are refused with 409, and new versions of existing files are still accepted.
"""
import concurrent.futures as cf
import json
import sys
import urllib.error
import urllib.request

BASE = sys.argv[1]


def call(method, path, body=None, headers=None):
    req = urllib.request.Request(BASE + path, data=body, method=method, headers=headers or {})
    try:
        with urllib.request.urlopen(req) as r:
            return r.status, r.read()
    except urllib.error.HTTPError as e:
        return e.code, e.read()


_, raw = call("POST", "/api/s")
session = json.loads(raw)
code, limit = session["code"], session["max_files"]
print("session", code, "limit", limit)

total = limit + 20
with cf.ThreadPoolExecutor(max_workers=24) as pool:
    results = list(pool.map(lambda i: call("PUT", f"/api/s/{code}/upload?name=f{i:03d}.txt", b"x"), range(total)))

stored = sum(1 for s, _ in results if s == 201)
refused = [b for s, b in results if s == 409]
other = [s for s, _ in results if s not in (201, 409)]
print(f"stored {stored}, refused {len(refused)}, other {other}")
assert stored == limit, f"stored {stored}, wanted exactly {limit}"
assert len(refused) == total - limit and not other
assert b"maximum of" in refused[0]

_, listing = call("GET", f"/api/s/{code}/files")
assert len(json.loads(listing)["items"]) == limit

# A full session still takes new versions of files it has.
status, _ = call("PUT", f"/api/s/{code}/upload?name=f000.txt", b"newer")
assert status == 201, status
# The chunked path refuses a new name at the start.
status, _ = call("POST", f"/api/s/{code}/uploads", json.dumps({"name": "late.bin", "size": 5}).encode(), {"content-type": "application/json"})
assert status == 409, status

call("DELETE", f"/api/s/{code}")
print("PASS")
