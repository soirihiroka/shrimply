from __future__ import annotations

import errno
import os
import socket
import struct
import sys
import traceback
from fractions import Fraction
from pathlib import Path
from typing import cast

sys.path.insert(0, str(Path(__file__).parent.parent))

import msgpack
from cyclopts import App

_worker_arguments = tuple(sys.argv[1:])
sys.argv = [sys.argv[0]]

from shrimply_manim import worker
from shrimply_manim.ir_encoder import Encoder, packet, rational
from shrimply_manim.render_pool import PreparedFrame
from shrimply_manim.worker_types import Message, ParameterOverrides, WorkerArguments


app = App(name="shrimply-manim-worker")


def receive_parameters(sock: socket.socket) -> ParameterOverrides:
    length = struct.unpack(">I", receive_exact(sock, 4))[0]
    value = msgpack.unpackb(receive_exact(sock, length), raw=False)
    if not isinstance(value, dict):
        raise TypeError("Manim parameter overrides must be a map")
    return cast(ParameterOverrides, value)


def receive_exact(sock: socket.socket, size: int) -> bytes:
    value = bytearray()
    while len(value) < size:
        chunk = sock.recv(size - len(value))
        if not chunk:
            raise EOFError("Manim compiler socket closed")
        value.extend(chunk)
    return bytes(value)


def send(sock: socket.socket, value: Message) -> None:
    encoded = msgpack.packb(value, use_bin_type=True)
    sock.sendall(struct.pack(">I", len(encoded)) + encoded)


def connect_local(path: str) -> socket.socket:
    if sys.platform != "win32":
        sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        sock.connect(path)
        return sock

    import ctypes

    class SocketAddress(ctypes.Structure):
        _fields_ = [("family", ctypes.c_ushort), ("path", ctypes.c_char * 108)]

    encoded = os.fsencode(path)
    if b"\0" in encoded or len(encoded) >= 108:
        raise ValueError("WinSock AF_UNIX path must be fewer than 108 non-NUL bytes")
    address = SocketAddress(1, encoded)
    sock = socket.socket(1, socket.SOCK_STREAM)
    winsock = ctypes.WinDLL("Ws2_32")
    winsock.connect.argtypes = [
        ctypes.c_size_t,
        ctypes.POINTER(SocketAddress),
        ctypes.c_int,
    ]
    winsock.connect.restype = ctypes.c_int
    winsock.WSAGetLastError.restype = ctypes.c_int
    if winsock.connect(sock.fileno(), ctypes.byref(address), ctypes.sizeof(address)) == -1:
        error = winsock.WSAGetLastError()
        sock.close()
        raise OSError(error, f"WinSock AF_UNIX connect failed: {error}")
    return sock


def run(args: WorkerArguments) -> None:
    fps = Fraction(args.fps)
    sock = connect_local(args.socket)
    try:
        parameters = receive_parameters(sock)
        send(
            sock,
            packet(
                "progress",
                {"stage": "loading_scene", "completed": 0, "total": 0},
            ),
        )
        encoder = Encoder()
        frame_count = 0

        def emit_frame(index: int, position: Fraction, prepared: PreparedFrame) -> None:
            nonlocal frame_count
            if index != frame_count:
                raise RuntimeError(
                    f"Manim frame index {index} is not contiguous; expected {frame_count}"
                )
            frame = encoder.encode_frame(index, position, prepared)
            frame_count += 1
            resources = encoder.resources()
            if resources is not None:
                send(sock, packet("resources", resources))
            send(sock, packet("frames", {"frames": [frame]}))
            send(
                sock,
                packet(
                    "progress",
                    {
                        "stage": "streaming_frames",
                        "completed": frame_count,
                        "total": 0,
                    },
                ),
            )

        names, selected_name, parameters, render_is_current, samples = worker.load_scene(
            args.source,
            args.scene,
            args.width,
            args.height,
            fps,
            emit_frame,
            parameter_overrides=parameters,
        )
        duration = Fraction(frame_count) / fps
        send(
            sock,
            packet(
                "scene",
                {
                    "scene": selected_name,
                    "scenes": names,
                    "width": args.width,
                    "height": args.height,
                    "samples": samples,
                    "fps": rational(fps),
                    "duration": rational(duration),
                    "frame_count": frame_count,
                    "complete": True,
                    "render_is_current": render_is_current,
                    "parameters": msgpack.packb(parameters, use_bin_type=True),
                },
            ),
        )
        send(sock, packet("finished"))
    except Exception as exception:
        if (
            isinstance(exception, OSError)
            and exception.errno == getattr(errno, "ENEEDAUTH", None)
        ):
            exception.add_note(
                f"macOS could not authenticate the filesystem containing {args.source}. "
                "Open the file in Finder and authenticate or download it, then retry."
            )
        if isinstance(exception, ModuleNotFoundError) and exception.name == "manim":
            exception.add_note(
                "Shrimply uses ManimGL (`manimlib`), not Manim Community Edition "
                "(`manim`). Rewrite this scene for ManimGL and import it with "
                "`from manimlib import *`; changing only the import may not be "
                "sufficient because the APIs differ."
            )
        try:
            send(sock, packet("error", traceback.format_exc()))
        except OSError:
            pass
        raise
    finally:
        sock.close()


@app.default
def main(
    *,
    socket: str,
    source: Path,
    width: int,
    height: int,
    fps: str,
    scene: str = "",
) -> None:
    run(
        WorkerArguments(
            socket=socket,
            source=source,
            scene=scene,
            width=width,
            height=height,
            fps=fps,
        )
    )
    os._exit(0)


if __name__ == "__main__":
    app(_worker_arguments)
