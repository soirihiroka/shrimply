"""Print the native SM target for CUDA device 0, as used by the renderer."""

import ctypes
import sys

CU_DEVICE_ATTRIBUTE_COMPUTE_CAPABILITY_MAJOR = 75
CU_DEVICE_ATTRIBUTE_COMPUTE_CAPABILITY_MINOR = 76

try:
    cuda = (
        ctypes.WinDLL("nvcuda.dll")
        if sys.platform == "win32"
        else ctypes.CDLL("libcuda.so.1")
    )
except OSError as error:
    raise SystemExit(f"Could not load the NVIDIA CUDA driver: {error}") from error

device = ctypes.c_int()
major = ctypes.c_int()
minor = ctypes.c_int()
for name, arguments in (
    ("cuInit", (0,)),
    ("cuDeviceGet", (ctypes.byref(device), 0)),
    (
        "cuDeviceGetAttribute",
        (ctypes.byref(major), CU_DEVICE_ATTRIBUTE_COMPUTE_CAPABILITY_MAJOR, device),
    ),
    (
        "cuDeviceGetAttribute",
        (ctypes.byref(minor), CU_DEVICE_ATTRIBUTE_COMPUTE_CAPABILITY_MINOR, device),
    ),
):
    result = getattr(cuda, name)(*arguments)
    if result != 0:
        raise SystemExit(f"Could not detect native CUDA architecture: {name} returned {result}")

print(f"sm_{major.value}{minor.value}")
