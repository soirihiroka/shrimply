"""Exercise packaged native editors against a fresh loopback project server."""

import base64
import json
import os
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from urllib.error import HTTPError
from urllib.request import Request, urlopen


def await_state(read, ready):
    for _ in range(40):
        value = read()
        if ready(value):
            return value
        time.sleep(0.5)
    raise AssertionError(f"State did not converge: {value!r}")


def smoke(executable, server_executable, directory):
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        host, port = listener.getsockname()
    endpoint = f"http://{host}:{port}"
    server = None
    server_log = directory / "server.log"
    alice, bob = (directory / f"{name}.shrimp" for name in ("Alice", "Bob"))

    def cli(*args, project=None, password=None):
        command = [executable, *args]
        if project is not None:
            command.extend(["--project", project])
        # File redirection lets CLI commands finish even while a daemon survives.
        with tempfile.TemporaryFile() as out, tempfile.TemporaryFile() as err:
            result = subprocess.run(command, stdout=out, stderr=err, input=password, timeout=45)
            out.seek(0)
            reply = out.read()
            err.seek(0)
            if result.returncode or not reply:
                raise AssertionError((args[:2], result.returncode, reply, err.read()))
        response = json.loads(reply)
        assert response["success"], response
        return response.get("result")

    def start(password):
        nonlocal server
        with server_log.open("ab") as log:
            server = subprocess.Popen(
                [server_executable, f"{host}:{port}", directory / "projects.sqlite"],
                stdout=log, stderr=subprocess.STDOUT,
                env=dict(os.environ, SHRIMPLY_PROJECT_SERVER_USERNAME="editor",
                         SHRIMPLY_PROJECT_SERVER_PASSWORD=password),
            )

        def protocol():
            assert server.poll() is None, server_log.read_text()
            try:
                with urlopen(f"{endpoint}/v1", timeout=2) as response:
                    return json.load(response)["protocol"]
            except OSError:
                return None

        await_state(protocol, lambda version: version == 3)

    def stop():
        nonlocal server
        if server is not None:
            if server.poll() is None:
                server.terminate()
                try:
                    server.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    server.kill()
                    server.wait()
            server = None

    def sign_in(password):
        cli("project", "login", "--server", endpoint, "--username", "editor",
            password=(password + "\n").encode())

    def metadata(project):
        return cli("project", "get", project=project)

    def named(project, name):
        await_state(lambda: metadata(project), lambda result: result["project_name"] == name)

    try:
        start("first-ci-password")
        sign_in("first-ci-password")
        basic = base64.b64encode(b"editor:first-ci-password").decode()
        login = Request(f"{endpoint}/v1/login", headers={"Authorization": f"Basic {basic}"}, method="POST")
        with urlopen(login, timeout=4) as response:
            token = json.load(response)["token"]
        created = cli("project", "create", "--server", endpoint, "--name", "Original",
                      "--fps", "30", project=alice)
        remote_id = created["location"]["id"]
        joined = cli("project", "open", "--server", endpoint, "--remote-project-id",
                     remote_id, project=bob)
        assert joined["ready"], joined
        cli("project", "set", "--name", "Shared", project=alice)
        named(bob, "Shared")
        revision = metadata(alice)["content_revision"]
        cli("player", "seek", "--frame", "30", project=bob)
        await_state(
            lambda: metadata(alice),
            lambda result: result["content_revision"] == revision
            and any(participant["position"]["seconds"] == {"numerator": 1, "denominator": 1}
                    for participant in result["shared_project"]["participants"]),
        )
        print("Remote edit and collaborator playhead verified", flush=True)

        owner_paths = [project.with_suffix(".shrimp.lock") for project in (alice, bob)]
        owners = [owner.read_bytes() for owner in owner_paths]
        stop()
        cli("project", "set", "--name", "Offline", project=bob)
        named(bob, "Offline")
        start("new-ci-password")
        old_session = Request(f"{endpoint}/v1/projects", headers={"Authorization": f"Bearer {token}"})
        try:
            with urlopen(old_session, timeout=4) as response:
                raise AssertionError(f"Rotated session still accepted: {response.status}")
        except HTTPError as error:
            with error:
                assert error.code == 401, error.code
        assert metadata(alice)["project_name"] == "Shared"
        sign_in("new-ci-password")
        named(alice, "Offline")
        assert cli("history", "undo", project=bob)["changed"]
        named(alice, "Shared")
        assert [owner.read_bytes() for owner in owner_paths] == owners
        print("Offline edit, credential rotation, and undo verified", flush=True)
    finally:
        try:
            for project in (alice, bob):
                if project.exists():
                    try:
                        cli("project", "close", project=project)
                    except (AssertionError, subprocess.TimeoutExpired) as error:
                        print(f"Cleanup failed for {project.name}: {error}", file=sys.stderr)
        finally:
            stop()


if __name__ == "__main__":
    app_path, server_path = (Path(arg).resolve(strict=True) for arg in sys.argv[1:])
    subprocess.run([app_path, "--version"], check=True, timeout=15)
    with tempfile.TemporaryDirectory() as temporary:
        smoke(app_path, server_path, Path(temporary))
