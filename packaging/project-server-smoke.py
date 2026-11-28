"""Exercise two sessions against the hosted project-server container."""
import base64
import hashlib
import json
import os
import sys
from urllib.error import HTTPError
from urllib.request import Request, urlopen
from uuid import uuid4

endpoint = sys.argv[1]
username = os.environ["SHRIMPLY_PROJECT_SERVER_USERNAME"]
password = os.environ["SHRIMPLY_PROJECT_SERVER_PASSWORD"]


def request(method, path, authorization="", *, status=200, data=None, headers=None):
    headers = dict(headers or {})
    if authorization:
        headers["Authorization"] = authorization
    if data is not None and not isinstance(data, bytes):
        data = json.dumps(data).encode()
        headers["Content-Type"] = "application/json"
    query = Request(endpoint + path, data=data, headers=headers, method=method)
    try:
        response = urlopen(query, timeout=5)
    except HTTPError as error:
        response = error
    with response:
        body = response.read()
        assert response.status == status, (method, path, response.status, body)
        if method == "GET" and path == "/projects" and status == 200:
            assert response.headers["x-shrimply-project-server"] == "project-server-smoke"
        return json.loads(body) if response.headers.get_content_type() == "application/json" else body


def sign_in(password, status=200):
    basic = base64.b64encode(f"{username}:{password}".encode()).decode()
    return request("POST", "/login", f"Basic {basic}", status=status)


def publish(client, revision, document, media=(), *, status=200, operation=None):
    return request("POST", project + "/snapshots", client, status=status, data={
        "base_revision": revision,
        "operation_id": operation or str(uuid4()),
        "operation_digest": hashlib.sha256(json.dumps(document, sort_keys=True).encode()).hexdigest(),
        "project": document,
        "media": list(media),
    })


sign_in("wrong", 401)
request("GET", "/projects", status=401)
alice, bob = ("Bearer " + sign_in(password)["token"] for _ in range(2))
assert request("GET", "/projects", alice) == []

project = "/projects/" + str(uuid4())
source = {"name": "Smoke"}
assert request("POST", project, alice, status=201, data=source)["revision"] == 0
presence = [{"id": str(uuid4()), "name": name, "position": position}
            for name, position in [("Alice", "1/2"), ("Bob", "3/4")]]
request("POST", project + "/sync", alice, data={"revision": 0, "presence": presence[0]})
seen_by_bob = request("POST", project + "/sync", bob, data={"revision": 0, "presence": presence[1]})
assert presence[0] in seen_by_bob["participants"] and seen_by_bob["version"] is None
assert seen_by_bob["server_version"] == request("GET", "")

media = b"project audio segment"
digest = hashlib.sha256(media).hexdigest()
blob = f"{project}/blobs/{digest}"
operation = str(uuid4())
alice_edit = source | {"caption": "Hello"}
publish(alice, 0, alice_edit, [digest], operation=operation, status=400)
for offset, block in [(0, media[:7]), (7, media[7:])]:
    receipt = request("PATCH", blob, alice, data=block, headers={
        "Upload-Offset": str(offset), "Upload-Length": str(len(media)),
    })
    assert receipt["offset"] == offset + len(block)
    assert receipt["complete"] == (offset != 0)
    assert request("GET", blob + "/upload", bob) == receipt

assert publish(alice, 0, alice_edit, [digest], operation=operation)["revision"] == 1
assert publish(alice, 0, alice_edit, [digest], operation=operation)["revision"] == 1
assert request("GET", f"{project}/operations/{operation}", bob)["revision"] == 1
assert request("GET", blob, bob, status=206, headers={"Range": "bytes=4-11"}) == media[4:12]
assert publish(bob, 0, source | {"label": "Edited"}, status=409)["revision"] == 1
latest = request("POST", project + "/sync", bob, data={"revision": 0, "presence": presence[1]})
assert latest["version"] == {"revision": 1, "project": alice_edit}
assert presence[0] in latest["participants"]
merged = latest["version"]["project"] | {"label": "Edited"}
assert publish(bob, 1, merged, [digest])["revision"] == 2
assert request("GET", project, alice) == {"revision": 2, "project": merged}
assert request("GET", "/projects", alice)[0]["opened_at"] > 0
assert request("DELETE", project, alice)
request("GET", project, bob, status=404)
