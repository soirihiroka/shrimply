"""Exercise packaged native editors against a fresh loopback project server."""

import base64
import hashlib
import json
import os
import socket
import subprocess
import sys
import tempfile
import time
import wave
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


def smoke(executable, server_executable, directory, alternate_servers):
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        host, port = listener.getsockname()
    endpoint = f"http://{host}:{port}"
    server = None
    server_log = directory / "server.log"
    alice, bob = (directory / f"{name}.shrimp" for name in ("Alice", "Bob"))

    def cli(*args, project=None, password=None, expected_error=None):
        command = [executable, *args]
        if project is not None:
            command.extend(["--project", project])
        # File redirection lets CLI commands finish even while a daemon survives.
        with tempfile.TemporaryFile() as out, tempfile.TemporaryFile() as err:
            result = subprocess.run(command, stdout=out, stderr=err, input=password, timeout=45)
            out.seek(0)
            reply = out.read()
            err.seek(0)
            if expected_error is not None:
                response = json.loads(reply)
                assert result.returncode and not response["success"], response
                assert expected_error in response["error"]["message"], response
                return None
            if result.returncode or not reply:
                raise AssertionError((args[:2], result.returncode, reply, err.read()))
        response = json.loads(reply)
        assert response["success"], response
        return response.get("result")

    def start(password, path=server_executable):
        nonlocal server
        with server_log.open("ab") as log:
            server = subprocess.Popen(
                [path, f"{host}:{port}", directory / "projects.sqlite"],
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

        if alternate_servers:
            audio = directory / "shared.wav"
            with wave.open(str(audio), "wb") as source:
                source.setparams((1, 2, 44100, 0, "NONE", "not compressed"))
                source.writeframes(bytes(44100 * 2 * 50))
            blob = "blobs/" + hashlib.sha256(audio.read_bytes()).hexdigest() + ".wav"
            body = json.dumps({"files": [{"source": str(audio)}], "frame": 0, "collision": "new_track"})
            cli("clips", "insert-files", "--body", body, "--dry-run", project=alice)
            cli("clips", "insert-files", "--body", body, project=alice)
            clips = await_state(lambda: cli("clips", "list", project=bob)["clips"],
                                lambda clips: any(clip["asset_path"] == blob for clip in clips))
            clip_id = next(clip["address"]["item_id"] for clip in clips if clip["asset_path"] == blob)

            def check_media():
                info = cli("clips", "inspect", "--item-id", clip_id, project=bob)
                assert info["source_error"] is None, info
                assert info["source"]["file"]["byte_size"] == audio.stat().st_size
                assert info["source"]["file"]["canonical_path"] is None
                streams = [stream.get("audio") or {} for stream in info["source"]["streams"]]
                assert any(audio.get("sample_rate") == 44100 and audio.get("channels") == 1 for audio in streams)

            check_media()
            stop()
            closed = cli("project", "close", project=bob)
            assert closed["closed"] and closed["saved"]
            check_media()
            current_owners = [owner.read_bytes() for owner in owner_paths]
            assert current_owners[0] == owners[0] and current_owners[1] != owners[1]
            owners = current_owners
            start("new-ci-password")
            print("Shared media uploaded and inspected from a fresh offline daemon", flush=True)

        initial_revision = metadata(alice)["content_revision"]
        for alternate, guidance in zip(alternate_servers, (
            "Update the project server to match Shrimply.",
            "Update Shrimply to match this project server.",
        ), strict=bool(alternate_servers)):
            version = subprocess.check_output([alternate, "--version"], text=True).split()[-1]
            stop()
            start("new-ci-password", alternate)
            cli("project", "get", project=alice, expected_error=guidance)
            observed = await_state(
                lambda: cli("project", "get", "--force-version", project=alice),
                lambda result: (result["shared_project"].get("server_version") or {}).get("version") == version,
            )
            assert observed["project_name"] == "Shared" and observed["content_revision"] == initial_revision
            assert [owner.read_bytes() for owner in owner_paths] == owners
            print(f"Observed live server {version}, correct update guidance, and preserved Force Open", flush=True)
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
    app_path, server_path, *alternate_servers = (Path(arg).resolve(strict=True) for arg in sys.argv[1:])
    subprocess.run([app_path, "--version"], check=True, timeout=15)
    with tempfile.TemporaryDirectory() as temporary:
        smoke(app_path, server_path, Path(temporary), alternate_servers)
