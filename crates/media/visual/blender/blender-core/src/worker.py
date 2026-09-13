from __future__ import annotations

import argparse
from fractions import Fraction
import json
import math
import os
import socket
import struct
import sys
import tempfile
import traceback
from typing import Any

arguments: tuple[str, ...] = tuple(sys.argv[sys.argv.index("--") + 1 :])
sys.argv = [sys.argv[0]]

import bpy
import gpu


def send(sock: socket.socket, value: dict[str, Any]) -> None:
    payload = json.dumps(value, separators=(",", ":")).encode()
    header = struct.pack(">I", len(payload))
    sock.sendall(header)
    sock.sendall(payload)


def receive(sock: socket.socket) -> dict[str, Any]:
    header = receive_exact(sock, 4)
    return json.loads(receive_exact(sock, struct.unpack(">I", header)[0]))


def receive_exact(sock: socket.socket, size: int) -> bytes:
    value = bytearray()
    while len(value) < size:
        chunk = sock.recv(size - len(value))
        if not chunk:
            raise EOFError("Shrimply closed the Blender worker socket")
        value.extend(chunk)
    return bytes(value)


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


def metadata() -> list[dict[str, Any]]:
    scenes: list[dict[str, Any]] = []
    for scene in bpy.data.scenes:
        cameras = [obj.name for obj in scene.objects if obj.type == "CAMERA"]
        fps = Fraction(scene.render.fps) / Fraction(str(scene.render.fps_base))
        scenes.append({
            "name": scene.name,
            "view_layers": [layer.name for layer in scene.view_layers],
            "cameras": cameras,
            "active_view_layer": scene.view_layers[0].name if scene.view_layers else "",
            "active_camera": scene.camera.name if scene.camera else (cameras[0] if cameras else ""),
            "frame_start": scene.frame_start,
            "frame_end": scene.frame_end,
            "fps_numerator": fps.numerator,
            "fps_denominator": fps.denominator,
        })
    return scenes


def read_targa(path: str, width: int, height: int) -> bytes:
    with open(path, "rb") as image:
        header = image.read(18)
        if len(header) != 18:
            raise RuntimeError("Blender produced an incomplete Targa header")
        identifier_length, color_map, image_type = header[:3]
        rendered_width, rendered_height = struct.unpack_from("<HH", header, 12)
        pixel_depth = header[16]
        if color_map != 0 or image_type != 2 or pixel_depth != 32:
            raise RuntimeError("Blender produced an unsupported Targa image")
        if rendered_width != width or rendered_height != height:
            raise RuntimeError(
                f"Blender produced {rendered_width}x{rendered_height}, expected {width}x{height}"
            )
        if identifier_length:
            image.seek(identifier_length, os.SEEK_CUR)
        pixels = image.read(width * height * 4)
    if len(pixels) != width * height * 4:
        raise RuntimeError("Blender produced incomplete frame pixels")
    return pixels


def initialize_background_gpu(
    scene: bpy.types.Scene,
    view_layer: bpy.types.ViewLayer,
    window: bpy.types.Window,
    output: str,
) -> None:
    try:
        gpu.platform.backend_type_get()
        return
    except SystemError:
        pass

    render = scene.render
    original = (
        render.engine,
        render.resolution_x,
        render.resolution_y,
        render.resolution_percentage,
        render.filepath,
        render.use_file_extension,
        render.image_settings.file_format,
        render.image_settings.color_mode,
        render.image_settings.color_depth,
    )
    eevee = getattr(scene, "eevee", None)
    eevee_original = None
    if eevee is not None:
        eevee_original = (eevee.taa_render_samples, eevee.use_raytracing)
    try:
        render.engine = "BLENDER_EEVEE"
        render.resolution_x = 4
        render.resolution_y = 4
        render.resolution_percentage = 100
        render.filepath = output
        render.use_file_extension = False
        render.image_settings.file_format = "TARGA_RAW"
        render.image_settings.color_mode = "RGBA"
        render.image_settings.color_depth = "8"
        if eevee is not None:
            eevee.taa_render_samples = 1
            eevee.use_raytracing = False
        with bpy.context.temp_override(
            window=window, scene=scene, view_layer=view_layer
        ):
            result = bpy.ops.render.render(
                "EXEC_DEFAULT",
                write_still=True,
                layer=view_layer.name,
                scene=scene.name,
            )
        if result != {"FINISHED"}:
            raise RuntimeError(f"Blender GPU initialization render failed: {result}")
        gpu.platform.backend_type_get()
    finally:
        (
            render.engine,
            render.resolution_x,
            render.resolution_y,
            render.resolution_percentage,
            render.filepath,
            render.use_file_extension,
            render.image_settings.file_format,
            render.image_settings.color_mode,
            render.image_settings.color_depth,
        ) = original
        if eevee is not None and eevee_original is not None:
            eevee.taa_render_samples, eevee.use_raytracing = eevee_original


def viewport_context(
    window: bpy.types.Window,
) -> tuple[bpy.types.Area, bpy.types.SpaceView3D, bpy.types.Region]:
    area = next(
        (area for area in window.screen.areas if area.type == "VIEW_3D"),
        None,
    )
    if area is None:
        area = max(window.screen.areas, key=lambda area: area.width * area.height)
        area.type = "VIEW_3D"
    region = next(region for region in area.regions if region.type == "WINDOW")
    return area, area.spaces.active, region


def render_viewport(
    offscreen: gpu.types.GPUOffScreen,
    scene: bpy.types.Scene,
    view_layer: bpy.types.ViewLayer,
    camera: bpy.types.Object,
    window: bpy.types.Window,
    method: str,
    width: int,
    height: int,
    warmup: bool,
) -> bytes:
    area, view_3d, region = viewport_context(window)
    shading = view_3d.shading
    original = (shading.type, shading.use_scene_lights, shading.use_scene_world)
    eevee = getattr(scene, "eevee", None)
    raytracing = eevee.use_raytracing if eevee is not None else None
    try:
        shading.type = "SOLID" if method == "solid" else "MATERIAL"
        if method == "material_preview":
            shading.use_scene_lights = False
            shading.use_scene_world = False
            if eevee is not None:
                eevee.use_raytracing = False
        window.scene = scene
        window.view_layer = view_layer
        with bpy.context.temp_override(
            window=window,
            area=area,
            region=region,
            scene=scene,
            view_layer=view_layer,
        ):
            projection_matrix = camera.calc_matrix_camera(
                bpy.context.evaluated_depsgraph_get(), x=width, y=height
            )
            for _ in range(2 if warmup else 1):
                offscreen.draw_view3d(
                    scene,
                    view_layer,
                    view_3d,
                    region,
                    camera.matrix_world.inverted(),
                    projection_matrix,
                    do_color_management=True,
                    draw_background=True,
                )
        buffer = offscreen.texture_color.read()
        buffer.dimensions = width * height * 4
        return bytes(buffer)
    finally:
        shading.type, shading.use_scene_lights, shading.use_scene_world = original
        if eevee is not None and raytracing is not None:
            eevee.use_raytracing = raytracing


def run(sock: socket.socket) -> None:
    window = bpy.context.window
    if window is None:
        raise RuntimeError("Blender did not create a background context")

    send(sock, {"kind": "hello", "protocol": 3})
    send(sock, {"kind": "metadata", "scenes": metadata()})
    temporary_root = (
        "/dev/shm"
        if os.path.isdir("/dev/shm") and os.access("/dev/shm", os.W_OK)
        else None
    )
    offscreen: gpu.types.GPUOffScreen | None = None
    offscreen_size: tuple[int, int] | None = None
    offscreen_method: str | None = None
    try:
        with tempfile.TemporaryDirectory(
            prefix="shrimply-blender-", dir=temporary_root
        ) as directory:
            output = os.path.join(directory, "frame.tga")
            while True:
                request = receive(sock)
                kind = request.get("kind")
                if kind == "shutdown":
                    return
                if kind != "render":
                    raise RuntimeError(f"unknown Blender worker request {kind!r}")
                scene = bpy.data.scenes[request["scene"]]
                view_layer = scene.view_layers[request["view_layer"]]
                camera = scene.objects[request["camera"]]
                if camera.type != "CAMERA":
                    raise RuntimeError(f"{camera.name!r} is not a camera")
                width = max(1, int(request["width"]))
                height = max(1, int(request["height"]))
                fps = Fraction(scene.render.fps) / Fraction(str(scene.render.fps_base))
                position = Fraction(request["time_numerator"], request["time_denominator"])
                frame = Fraction(scene.frame_start) + position * fps
                whole = math.floor(frame)
                window.scene = scene
                window.view_layer = view_layer
                scene.camera = camera
                scene.frame_set(whole, subframe=float(frame - whole))
                method = request["method"]
                if method not in ("solid", "material_preview", "scene_renderer"):
                    raise RuntimeError(f"unknown Blender render method {method!r}")
                if method == "scene_renderer":
                    render = scene.render
                    original = (
                        render.resolution_x,
                        render.resolution_y,
                        render.resolution_percentage,
                        render.filepath,
                        render.use_file_extension,
                        render.image_settings.file_format,
                        render.image_settings.color_mode,
                        render.image_settings.color_depth,
                    )
                    try:
                        render.resolution_x = width
                        render.resolution_y = height
                        render.resolution_percentage = 100
                        render.filepath = output
                        render.use_file_extension = False
                        render.image_settings.file_format = "TARGA_RAW"
                        render.image_settings.color_mode = "RGBA"
                        render.image_settings.color_depth = "8"
                        with bpy.context.temp_override(
                            window=window, scene=scene, view_layer=view_layer
                        ):
                            result = bpy.ops.render.render(
                                "EXEC_DEFAULT",
                                write_still=True,
                                layer=view_layer.name,
                                scene=scene.name,
                            )
                        if result != {"FINISHED"}:
                            raise RuntimeError(f"Blender render failed: {result}")
                        pixels = read_targa(output, width, height)
                    finally:
                        (
                            render.resolution_x,
                            render.resolution_y,
                            render.resolution_percentage,
                            render.filepath,
                            render.use_file_extension,
                            render.image_settings.file_format,
                            render.image_settings.color_mode,
                            render.image_settings.color_depth,
                        ) = original
                    pixel_format = "bgra"
                else:
                    initialize_background_gpu(scene, view_layer, window, output)
                    if offscreen_size != (width, height):
                        if offscreen is not None:
                            offscreen.free()
                        offscreen = gpu.types.GPUOffScreen(width, height, format="RGBA8")
                        offscreen_size = (width, height)
                        offscreen_method = None
                    pixels = render_viewport(
                        offscreen,
                        scene,
                        view_layer,
                        camera,
                        window,
                        method,
                        width,
                        height,
                        offscreen_method != method,
                    )
                    offscreen_method = method
                    pixel_format = "rgba"
                send(sock, {
                    "kind": "frame",
                    "width": width,
                    "height": height,
                    "byte_len": len(pixels),
                    "pixel_format": pixel_format,
                })
                sock.sendall(pixels)
    finally:
        if offscreen is not None:
            offscreen.free()


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--socket", required=True)
    args = parser.parse_args(arguments)
    sock = connect_local(args.socket)
    try:
        run(sock)
    except Exception:
        try:
            send(sock, {"kind": "error", "message": traceback.format_exc()})
        except OSError:
            pass
        raise
    finally:
        sock.close()
    os._exit(0)


main()
