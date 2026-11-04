import gc
import logging
import os
import sys
import tempfile
import types
from importlib.machinery import ModuleSpec
from pathlib import Path

import msgspec
import numpy as np
import torch
from pydantic import ValidationError

from api import gpu
from api.tracking_3d.protocol import (
    ANALYSIS_REQUEST_VALIDATOR,
    ARCHIVE_MAGIC,
    MAXIMUM_HEADER_BYTES,
    MAXIMUM_JPEG_BYTES,
    AnalysisRequest,
    CameraEvent,
    ErrorEvent,
    ModelId,
    ProgressEvent,
    Projection,
    ResultEvent,
    encode_event,
)

logger = logging.getLogger("shrimply.3dtracking.worker")
SUBMAP_SIZE = 4
OVERLAPPING_WINDOW_SIZE = 1
MAXIMUM_LOOPS = 1
CONFIDENCE_THRESHOLD = 25.0
LOOP_CLOSURE_THRESHOLD = 0.95
SALAD_CHECKPOINT_URL = (
    "https://github.com/serizba/salad/releases/download/v1.0.0/dino_salad.ckpt"
)
type ColmapOptionValue = bool | int | float | str | dict[str, bool | int]


def _read_exact(file, length: int) -> bytes:
    value = file.read(length)
    if len(value) != length:
        raise ValueError("Truncated 3D tracking archive")
    return value


def unpack_archive(path: str, image_path: Path) -> tuple[AnalysisRequest, list[Path]]:
    with open(path, "rb") as file:
        if _read_exact(file, len(ARCHIVE_MAGIC)) != ARCHIVE_MAGIC:
            raise ValueError("Invalid 3D tracking archive magic")
        header_length = int.from_bytes(_read_exact(file, 8), "little")
        if header_length <= 0 or header_length > MAXIMUM_HEADER_BYTES:
            raise ValueError("Invalid 3D tracking archive header length")
        try:
            request = ANALYSIS_REQUEST_VALIDATOR.validate_python(
                msgspec.msgpack.decode(_read_exact(file, header_length))
            )
        except (msgspec.DecodeError, ValidationError) as exception:
            raise ValueError(
                f"Invalid 3D tracking analysis header: {exception}"
            ) from exception
        images: list[Path] = []
        seen: set[int] = set()
        for _ in range(request.frame_count):
            frame_index = int.from_bytes(_read_exact(file, 8), "little")
            length = int.from_bytes(_read_exact(file, 8), "little")
            if frame_index >= request.frame_count or frame_index in seen:
                raise ValueError("Invalid or duplicate 3D tracking frame index")
            if length > MAXIMUM_JPEG_BYTES:
                raise ValueError("Invalid 3D tracking JPEG length")
            seen.add(frame_index)
            if length == 0:
                continue
            image = image_path / f"frame_{frame_index:010}.jpg"
            image.write_bytes(_read_exact(file, length))
            images.append(image)
        if len(seen) != request.frame_count:
            raise ValueError("3D tracking archive is missing frame records")
        if file.read(1):
            raise ValueError("Unexpected trailing 3D tracking archive data")
    if len(images) < 2:
        raise ValueError("3D tracking requires at least two visible frames")
    return request, images


def progress(connection, message: str, completed: int, total: int) -> None:
    connection.send_bytes(
        encode_event(
            ProgressEvent(
                message=message,
                completed_frames=completed,
                total_frames=total,
            )
        )
    )


def colmap_options(pycolmap, request: AnalysisRequest, device_index: str | None):
    quality = request.quality
    assert quality is not None
    extraction_presets: dict[str, dict[str, ColmapOptionValue]] = {
        "low": {
            "max_image_size": 1000,
            "sift": {"max_num_features": 2048},
        },
        "medium": {
            "max_image_size": 1600,
            "sift": {"max_num_features": 4096},
        },
        "high": {
            "max_image_size": 2400,
            "sift": {
                "max_num_features": 8192,
                "estimate_affine_shape": True,
            },
        },
        "extreme": {
            "sift": {
                "max_num_features": 8192,
                "estimate_affine_shape": True,
                "domain_size_pooling": True,
            }
        },
    }
    extraction_values = extraction_presets[quality].copy()
    matching_values: dict[str, ColmapOptionValue] = {
        "guided_matching": quality in ("high", "extreme"),
    }
    pairing_values: dict[str, ColmapOptionValue] = {}
    mapping_values: dict[str, ColmapOptionValue] = {
        "ba_local_max_num_iterations": {
            "low": 12,
            "medium": 16,
            "high": 30,
            "extreme": 40,
        }[quality],
        "ba_local_max_refinements": 3 if quality in ("high", "extreme") else 2,
        "ba_global_max_num_iterations": {
            "low": 25,
            "medium": 33,
            "high": 75,
            "extreme": 100,
        }[quality],
        "ba_global_frames_ratio": {
            "low": 1.32,
            "medium": 1.21,
        }.get(quality, 1.1),
        "ba_global_points_ratio": {
            "low": 1.32,
            "medium": 1.21,
        }.get(quality, 1.1),
        "ba_global_max_refinements": 2 if quality in ("low", "medium") else 5,
    }
    extraction_values["use_gpu"] = device_index is not None
    matching_values["use_gpu"] = device_index is not None
    if device_index is not None:
        extraction_values["gpu_index"] = device_index
        matching_values["gpu_index"] = device_index
        mapping_values["ba_use_gpu"] = True
        mapping_values["ba_gpu_index"] = device_index
    return (
        pycolmap.FeatureExtractionOptions(extraction_values),
        pycolmap.FeatureMatchingOptions(matching_values),
        pycolmap.SequentialPairingOptions(pairing_values),
        pycolmap.IncrementalPipelineOptions(mapping_values),
    )


def analyze_colmap(
    connection,
    pycolmap,
    request: AnalysisRequest,
    images: list[Path],
    work: Path,
    device_name: str,
) -> int:
    database = work / "database.db"
    models = work / "models"
    models.mkdir()
    device_index = (
        device_name.removeprefix("cuda:") if device_name.startswith("cuda:") else None
    )
    device = pycolmap.Device.cuda if device_index is not None else pycolmap.Device.cpu
    extraction, matching, pairing, mapping = colmap_options(
        pycolmap, request, device_index
    )
    assert request.camera_model is not None
    camera_model = {
        "simple_radial": "SIMPLE_RADIAL",
        "pinhole": "PINHOLE",
        "open_cv": "OPENCV",
        "open_cv_fisheye": "OPENCV_FISHEYE",
        "equirectangular": "EQUIRECTANGULAR",
    }[request.camera_model]
    progress(connection, "Extracting COLMAP features...", 0, request.frame_count)
    pycolmap.extract_features(
        database,
        images[0].parent,
        image_names=[image.name for image in images],
        camera_mode=pycolmap.CameraMode.SINGLE,
        reader_options=pycolmap.ImageReaderOptions(camera_model=camera_model),
        extraction_options=extraction,
        device=device,
    )
    progress(connection, "Matching COLMAP frames...", 0, request.frame_count)
    pycolmap.match_sequential(
        database,
        matching_options=matching,
        pairing_options=pairing,
        device=device,
    )
    registered = 0

    def registered_image() -> None:
        nonlocal registered
        registered += 1
        progress(
            connection,
            "Reconstructing COLMAP cameras...",
            min(registered, request.frame_count),
            request.frame_count,
        )

    reconstructions = pycolmap.incremental_mapping(
        database,
        images[0].parent,
        models,
        options=mapping,
        next_image_callback=registered_image,
    )
    if not reconstructions:
        raise ValueError("COLMAP did not produce a reconstruction")
    reconstruction = max(
        reconstructions.values(), key=lambda value: value.num_reg_images()
    )
    if reconstruction.num_reg_images() < 2:
        raise ValueError("COLMAP registered fewer than two cameras")
    count = 0
    for image_id in reconstruction.reg_image_ids():
        image = reconstruction.image(image_id)
        camera = image.camera
        if camera is None:
            raise ValueError("Registered COLMAP image has no camera")
        frame_index = int(Path(image.name).stem.removeprefix("frame_"))
        pose = image.cam_from_world()
        projection: Projection
        match camera.model_name:
            case "OPENCV_FISHEYE":
                projection = "fisheye"
            case "EQUIRECTANGULAR":
                projection = "equirectangular"
            case _:
                projection = "perspective"
        focal_y = (
            None if projection == "equirectangular" else float(camera.focal_length_y)
        )
        connection.send_bytes(
            encode_event(
                CameraEvent(
                    frame_index=frame_index,
                    camera_from_world_rotation=pose.rotation.quat.tolist(),
                    camera_from_world_translation=pose.translation.tolist(),
                    projection=projection,
                    image_width=int(camera.width),
                    image_height=int(camera.height),
                    focal_y=focal_y,
                )
            )
        )
        count += 1
    return count


class HeadlessViewer:
    pass


def install_headless_vggt_modules() -> None:
    open3d = types.ModuleType("open3d")
    open3d.__spec__ = ModuleSpec("open3d", loader=None)
    sys.modules.setdefault("open3d", open3d)
    matplotlib = types.ModuleType("matplotlib")
    matplotlib.__path__ = []
    matplotlib.__spec__ = ModuleSpec("matplotlib", loader=None, is_package=True)
    pyplot = types.ModuleType("matplotlib.pyplot")
    colors = types.ModuleType("matplotlib.colors")
    color_maps = types.ModuleType("matplotlib.cm")
    setattr(matplotlib, "pyplot", pyplot)
    setattr(matplotlib, "colors", colors)
    setattr(matplotlib, "cm", color_maps)
    sys.modules.setdefault("matplotlib", matplotlib)
    sys.modules.setdefault("matplotlib.pyplot", pyplot)
    sys.modules.setdefault("matplotlib.colors", colors)
    sys.modules.setdefault("matplotlib.cm", color_maps)
    toolkits = types.ModuleType("mpl_toolkits")
    toolkits.__path__ = []
    toolkits.__spec__ = ModuleSpec("mpl_toolkits", loader=None, is_package=True)
    mplot3d = types.ModuleType("mpl_toolkits.mplot3d")
    setattr(mplot3d, "Axes3D", HeadlessViewer)
    sys.modules.setdefault("mpl_toolkits", toolkits)
    sys.modules.setdefault("mpl_toolkits.mplot3d", mplot3d)
    viewer = types.ModuleType("vggt_slam.viewer")
    viewer.__spec__ = ModuleSpec("vggt_slam.viewer", loader=None)
    setattr(viewer, "Viewer", HeadlessViewer)
    sys.modules.setdefault("vggt_slam.viewer", viewer)


def load_vggt_slam(device: torch.device):
    __import__("pytorch_lightning")
    install_headless_vggt_modules()
    from huggingface_hub import hf_hub_download
    from vggt.models.vggt import VGGT
    from vggt_slam.solver import Solver

    torch.cuda.set_device(device)
    dtype = gpu.inference_dtype(device)
    persistent_salad_checkpoint = (
        Path.home() / ".cache" / "huggingface" / "dino_salad.ckpt"
    )
    if not persistent_salad_checkpoint.exists():
        persistent_salad_checkpoint.parent.mkdir(parents=True, exist_ok=True)
        torch.hub.download_url_to_file(
            SALAD_CHECKPOINT_URL, str(persistent_salad_checkpoint)
        )
    salad_checkpoint = Path(torch.hub.get_dir()) / "checkpoints" / "dino_salad.ckpt"
    if not salad_checkpoint.exists():
        salad_checkpoint.parent.mkdir(parents=True, exist_ok=True)
        salad_checkpoint.symlink_to(persistent_salad_checkpoint)
    model = VGGT()
    model.load_state_dict(
        torch.load(
            hf_hub_download(repo_id="facebook/VGGT-1B", filename="model.pt"),
            map_location="cpu",
            weights_only=True,
        )
    )
    model.eval().to(device=device, dtype=dtype)
    camera_head = model.camera_head
    depth_head = model.depth_head
    assert camera_head is not None and depth_head is not None

    def infer(images: torch.Tensor, compute_similarity: bool = False):
        if images.ndim == 4:
            images = images.unsqueeze(0)
        images = images.to(dtype=dtype)
        with torch.inference_mode(), torch.autocast(device_type="cuda", dtype=dtype):
            (
                aggregated_tokens,
                patch_start_index,
                target_tokens,
                image_match_ratio,
            ) = model.aggregator(images, compute_similarity)
            pose_encoding = camera_head(aggregated_tokens)[-1]
            depth, depth_confidence = depth_head(
                aggregated_tokens,
                images=images,
                patch_start_idx=patch_start_index,
            )
        return {
            "pose_enc": pose_encoding,
            "depth": depth,
            "depth_conf": depth_confidence,
            "images": images,
            "target_tokens": target_tokens,
            "image_match_ratio": image_match_ratio,
        }

    solver = Solver(
        init_conf_threshold=CONFIDENCE_THRESHOLD,
        lc_thres=LOOP_CLOSURE_THRESHOLD,
        vis_imgs=False,
    )
    return infer, solver


def reset_solver(solver) -> None:
    from vggt_slam.frame_overlap import FrameTracker
    from vggt_slam.graph import PoseGraph
    from vggt_slam.map import GraphMap
    from vggt_slam.slam_utils import Accumulator

    solver.flow_tracker = FrameTracker()
    solver.map = GraphMap()
    solver.graph = PoseGraph()
    solver.current_working_submap = None
    solver.temp_count = 0
    solver.vggt_timer = Accumulator()
    solver.loop_closure_timer = Accumulator()
    solver.clip_timer = Accumulator()


def analyze_vggt_slam(
    connection, model, solver, request: AnalysisRequest, images: list[Path]
) -> int:
    from scipy.spatial.transform import Rotation
    from vggt_slam.slam_utils import decompose_camera

    progress(connection, "Analyzing VGGT-SLAM frames...", 0, len(images))
    reset_solver(solver)
    selected: list[str] = []
    processed = 0
    for position, image_path in enumerate(images):
        selected.append(str(image_path))
        last = position == len(images) - 1
        if len(selected) == SUBMAP_SIZE + OVERLAPPING_WINDOW_SIZE or (
            last and len(selected) >= 2
        ):
            predictions = solver.run_predictions(
                selected, model, MAXIMUM_LOOPS, None, None
            )
            solver.add_points(predictions)
            solver.graph.optimize()
            processed += len(selected) - (OVERLAPPING_WINDOW_SIZE if processed else 0)
            progress(
                connection,
                "Analyzing VGGT-SLAM frames...",
                min(processed, len(images)),
                len(images),
            )
            selected = selected[-OVERLAPPING_WINDOW_SIZE:]
    progress(
        connection,
        "Finalizing VGGT-SLAM trajectory...",
        len(images),
        len(images),
    )
    cameras: dict[int, CameraEvent] = {}
    for submap in solver.map.ordered_submaps_by_key():
        if submap.get_lc_status():
            continue
        projections = submap.get_all_poses_world(solver.graph, give_camera_mat=True)
        height, width = submap.get_all_frames().shape[-2:]
        for frame_id, projection in zip(
            submap.get_frame_ids(), projections, strict=True
        ):
            intrinsic, rotation, translation, _ = decompose_camera(projection)
            camera_to_world = np.eye(4)
            camera_to_world[:3, :3] = rotation
            camera_to_world[:3, 3] = translation
            camera_from_world = np.linalg.inv(camera_to_world)
            cameras[int(frame_id)] = CameraEvent(
                frame_index=int(frame_id),
                camera_from_world_rotation=Rotation.from_matrix(
                    camera_from_world[:3, :3]
                )
                .as_quat()
                .tolist(),
                camera_from_world_translation=camera_from_world[:3, 3].tolist(),
                projection="perspective",
                image_width=int(width),
                image_height=int(height),
                focal_y=float(intrinsic[1, 1]),
            )
    if len(cameras) < 2:
        raise ValueError("VGGT-SLAM produced fewer than two camera poses")
    for camera in cameras.values():
        connection.send_bytes(encode_event(camera))
    count = len(cameras)
    reset_solver(solver)
    return count


def run(connection, model_id: ModelId, device_name: str) -> None:
    device = torch.device(device_name)
    colmap_backend = None
    vggt_backend = None
    try:
        connection.send_bytes(
            encode_event(
                ProgressEvent(
                    message=f"Loading {model_id}...",
                    completed_frames=0,
                    total_frames=1,
                )
            )
        )
        if model_id == "colmap/colmap":
            import pycolmap

            colmap_backend = pycolmap
        else:
            if device.type != "cuda":
                raise ValueError("MIT-SPARK/VGGT-SLAM requires CUDA")
            vggt_backend = load_vggt_slam(device)
        logger.info("3D tracking backend ready model=%s device=%s", model_id, device)
    except Exception as exception:
        logger.exception("3D tracking worker initialization failed")
        connection.send_bytes(
            encode_event(
                ErrorEvent(code="worker_initialization_failed", message=str(exception))
            )
        )
        connection.close()
        return
    while True:
        try:
            path = connection.recv_bytes().decode()
        except EOFError:
            break
        camera_count = None
        try:
            with tempfile.TemporaryDirectory(
                prefix="shrimply-3dtracking-worker-"
            ) as directory:
                work = Path(directory)
                images_path = work / "images"
                images_path.mkdir()
                request, images = unpack_archive(path, images_path)
                if request.model != model_id:
                    raise ValueError(
                        "3D tracking archive backend does not match worker"
                    )
                if model_id == "colmap/colmap":
                    assert colmap_backend is not None
                    camera_count = analyze_colmap(
                        connection,
                        colmap_backend,
                        request,
                        images,
                        work,
                        device_name,
                    )
                else:
                    assert vggt_backend is not None
                    model, solver = vggt_backend
                    camera_count = analyze_vggt_slam(
                        connection, model, solver, request, images
                    )
        except Exception as exception:
            logger.exception("3D tracking analysis failed path=%s", path)
            connection.send_bytes(
                encode_event(ErrorEvent(code="analysis_failed", message=str(exception)))
            )
        finally:
            gc.collect()
            if device.type == "cuda":
                torch.cuda.empty_cache()
        if camera_count is not None:
            connection.send_bytes(encode_event(ResultEvent(camera_count=camera_count)))
    logger.info("3D tracking worker pid=%d shutting down", os.getpid())
    connection.close()
