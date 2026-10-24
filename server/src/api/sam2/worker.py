import gc
import logging
import os
from io import BytesIO

import msgspec
import numpy as np
import torch
from pydantic import ValidationError
from transformers import Sam2VideoModel, Sam2VideoProcessor

from api.media import load_image

from api.sam2.protocol import (
    ANALYSIS_REQUEST_VALIDATOR,
    ARCHIVE_MAGIC,
    MAXIMUM_FRAME_BYTES,
    ErrorEvent,
    MaskEvent,
    ProgressEvent,
    ResultEvent,
    encode_event,
)

logger = logging.getLogger("shrimply.sam2.worker")
MASK_SIZE = 256
MODEL_SIZE = 1024
QUANTIZATION_SCALE = 16
def _read_exact(file, length: int) -> bytes:
    value = file.read(length)
    if len(value) != length:
        raise ValueError("Truncated SAM2 proxy archive")
    return value


def read_archive(path: str):
    with open(path, "rb") as file:
        if _read_exact(file, len(ARCHIVE_MAGIC)) != ARCHIVE_MAGIC:
            raise ValueError("Invalid SAM2 proxy archive magic")
        header_length = int.from_bytes(_read_exact(file, 8), "little")
        if header_length <= 0 or header_length > 1024 * 1024:
            raise ValueError("Invalid SAM2 proxy archive header length")
        try:
            request = ANALYSIS_REQUEST_VALIDATOR.validate_python(
                msgspec.msgpack.decode(_read_exact(file, header_length))
            )
        except (msgspec.DecodeError, ValidationError) as exception:
            raise ValueError(
                f"Invalid SAM2 analysis header: {exception}"
            ) from exception
        offsets: list[tuple[int, int]] = []
        for _ in range(request.frame_count):
            length = int.from_bytes(_read_exact(file, 8), "little")
            if length <= 0 or length > MAXIMUM_FRAME_BYTES:
                raise ValueError("Invalid SAM2 proxy frame length")
            offsets.append((file.tell(), length))
            file.seek(length, 1)
        if file.read(1):
            raise ValueError("Unexpected trailing SAM2 proxy archive data")
    return request, offsets


def load_frame(file, offset: tuple[int, int]) -> torch.Tensor:
    file.seek(offset[0])
    decoded = load_image(BytesIO(_read_exact(file, offset[1])))
    if decoded.size != (MODEL_SIZE, MODEL_SIZE):
        raise ValueError(f"SAM2 proxy frame has invalid size {decoded.size}")
    image = torch.from_numpy(np.array(decoded)).permute(2, 0, 1).contiguous()
    return image


def quantize_mask(mask: torch.Tensor) -> bytes:
    mask = mask.squeeze()
    if tuple(mask.shape) != (MASK_SIZE, MASK_SIZE):
        raise ValueError(f"SAM2 returned invalid mask shape {tuple(mask.shape)}")
    return (
        (mask.float() * QUANTIZATION_SCALE)
        .round()
        .clamp(torch.iinfo(torch.int8).min, torch.iinfo(torch.int8).max)
        .to(torch.int8)
        .cpu()
        .numpy()
        .tobytes()
    )


def make_session(processor, device: torch.device, dtype: torch.dtype, request):
    session = processor.init_video_session(
        inference_device=device,
        inference_state_device=device,
        video_storage_device="cpu",
        dtype=dtype,
    )
    points = [
        [[[point.x * MODEL_SIZE, point.y * MODEL_SIZE] for point in request.points]]
    ]
    labels = [[[point.label for point in request.points]]]
    box = None
    if request.box is not None:
        box = [
            [
                [
                    request.box.minimum[0] * MODEL_SIZE,
                    request.box.minimum[1] * MODEL_SIZE,
                    request.box.maximum[0] * MODEL_SIZE,
                    request.box.maximum[1] * MODEL_SIZE,
                ]
            ]
        ]
    processor.add_inputs_to_inference_session(
        inference_session=session,
        frame_idx=request.seed_frame,
        obj_ids=1,
        input_points=points if request.points else None,
        input_labels=labels if request.points else None,
        input_boxes=box,
        original_size=(MODEL_SIZE, MODEL_SIZE),
    )
    return session


def analyze_archive(connection, model, processor, device, dtype, path: str) -> int:
    request, offsets = read_archive(path)
    completed = 0
    with open(path, "rb") as file, torch.inference_mode():
        directions = (
            (range(request.seed_frame, request.frame_count), False),
            (range(request.seed_frame, -1, -1), True),
        )
        for indices, reverse in directions:
            session = make_session(processor, device, dtype, request)
            for frame_index in indices:
                duplicate_seed = reverse and frame_index == request.seed_frame
                image = load_frame(file, offsets[frame_index])
                inputs = processor(images=image, device="cpu", return_tensors="pt")
                frame = inputs.pixel_values[0].to(device=device, dtype=dtype)
                output = model(
                    inference_session=session,
                    frame_idx=frame_index,
                    frame=frame,
                    reverse=reverse,
                )
                if not duplicate_seed:
                    connection.send_bytes(
                        encode_event(
                            MaskEvent(
                                frame_index=frame_index,
                                mask=quantize_mask(output.pred_masks),
                            )
                        )
                    )
                    completed += 1
                    connection.send_bytes(
                        encode_event(
                            ProgressEvent(
                                message="Tracking objects...",
                                completed_frames=completed,
                                total_frames=request.frame_count,
                            )
                        )
                    )
                session.processed_frames.pop(frame_index, None)
    return completed


def run(connection, model_id: str, device_name: str) -> None:
    device = torch.device(device_name)
    bf16_supported = False
    if device.type == "cuda":
        with torch.cuda.device(device):
            bf16_supported = torch.cuda.is_bf16_supported()
    dtype = (
        torch.float32
        if device.type == "cpu"
        else torch.bfloat16
        if bf16_supported
        else torch.float16
    )
    try:
        connection.send_bytes(
            encode_event(
                ProgressEvent(
                    message="Loading SAM2 model...",
                    completed_frames=0,
                    total_frames=1,
                )
            )
        )
        processor = Sam2VideoProcessor.from_pretrained(model_id)
        model = Sam2VideoModel.from_pretrained(model_id, dtype=dtype)
        type(model).to(model, device)
        model.eval()
        logger.info(
            "SAM2 model ready model=%s device=%s dtype=%s", model_id, device, dtype
        )
    except Exception as exception:
        logger.exception("SAM2 worker initialization failed")
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
        completed = None
        try:
            completed = analyze_archive(
                connection, model, processor, device, dtype, path
            )
        except Exception as exception:
            logger.exception("SAM2 analysis failed path=%s", path)
            connection.send_bytes(
                encode_event(ErrorEvent(code="analysis_failed", message=str(exception)))
            )
        finally:
            gc.collect()
            if device.type == "cuda":
                torch.cuda.empty_cache()
        if completed is not None:
            connection.send_bytes(encode_event(ResultEvent(completed_frames=completed)))
    logger.info("SAM2 worker pid=%d shutting down", os.getpid())
    connection.close()
