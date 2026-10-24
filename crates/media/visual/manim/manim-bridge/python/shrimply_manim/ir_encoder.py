from __future__ import annotations

from dataclasses import dataclass, field
from fractions import Fraction

from shrimply_manim.render_pool import (
    PreparedDraw,
    PreparedFrame,
    PreparedPipelineState,
    PreparedTexture,
)
from shrimply_manim.worker_types import Message, MessageValue

SCHEMA_VERSION = 11

type PipelineKey = tuple[
    str,
    PreparedPipelineState,
    tuple[tuple[str, str], ...],
]


def packet(kind: str, payload: MessageValue = None) -> Message:
    body: Message = {"kind": kind}
    if payload is not None:
        body["payload"] = payload
    return {"schema_version": SCHEMA_VERSION, "body": body}


def rational(value: Fraction) -> Message:
    return {"numerator": value.numerator, "denominator": value.denominator}


def _numbers(value: MessageValue) -> list[float]:
    if not isinstance(value, list) or not all(
        isinstance(component, (int, float)) for component in value
    ):
        raise TypeError("Manim camera vector must contain only numbers")
    return [float(component) for component in value]


def _number(value: MessageValue) -> float:
    if not isinstance(value, (int, float)):
        raise TypeError("Manim camera scalar must be a number")
    return float(value)


@dataclass(slots=True)
class Encoder:
    next_resource_id: int = 0
    pending_pipelines: list[MessageValue] = field(default_factory=list)
    pending_geometry: list[MessageValue] = field(default_factory=list)
    pending_uniforms: list[MessageValue] = field(default_factory=list)
    pending_textures: list[MessageValue] = field(default_factory=list)
    pipeline_ids: dict[PipelineKey, int] = field(default_factory=dict)
    texture_ids: dict[tuple[str, str, int, int, int, bytes], int] = field(default_factory=dict)

    def _id(self) -> int:
        resource_id = self.next_resource_id
        self.next_resource_id += 1
        return resource_id

    def encode_frame(
        self,
        index: int,
        time: Fraction,
        prepared: PreparedFrame,
    ) -> Message:
        draws: list[MessageValue] = []
        for draw in prepared.draws:
            pipeline_id = self._pipeline(draw)

            geometry_id = self._id()
            self.pending_geometry.append(
                {
                    "id": geometry_id,
                    "bytes": draw.data,
                }
            )

            uniform_id = self._id()
            self.pending_uniforms.append({"id": uniform_id, "bytes": draw.uniforms})

            bindings: list[MessageValue] = []
            for texture in draw.textures:
                texture_id = self._texture(texture)
                bindings.append(
                    {
                        "binding": texture.binding,
                        "name": texture.name,
                        "texture": texture_id,
                    }
                )
            draws.append(
                {
                    "pipeline": pipeline_id,
                    "geometry": geometry_id,
                    "uniforms": uniform_id,
                    "vertex_count": draw.vertices,
                    "indices": list(draw.indices),
                    "textures": bindings,
                }
            )

        camera = prepared.camera_uniforms
        return {
            "index": index,
            "time": rational(time),
            "camera": {
                "view": _numbers(camera["view"]),
                "frame_scale": _number(camera["frame_scale"]),
                "frame_rescale_factors": _numbers(camera["frame_rescale_factors"]),
                "pixel_size": _number(camera["pixel_size"]),
                "camera_position": _numbers(camera["camera_position"]),
                "light_position": _numbers(camera["light_position"]),
                "background_rgba": list(prepared.background_rgba),
                "uniforms": prepared.camera_bytes,
            },
            "draws": draws,
        }

    def resources(self) -> Message | None:
        if not any(
            (
                self.pending_pipelines,
                self.pending_geometry,
                self.pending_textures,
                self.pending_uniforms,
            )
        ):
            return None
        result: Message = {
            "pipelines": self.pending_pipelines,
            "geometry": self.pending_geometry,
            "textures": self.pending_textures,
            "uniforms": self.pending_uniforms,
        }
        self.pending_pipelines = []
        self.pending_geometry = []
        self.pending_textures = []
        self.pending_uniforms = []
        return result

    def _texture(self, texture: PreparedTexture) -> int:
        key = (
            texture.kind, texture.filter, texture.width, texture.height,
            texture.layers, texture.pixels,
        )
        if key in self.texture_ids:
            return self.texture_ids[key]
        texture_id = self._id()
        self.pending_textures.append(
            {
                "id": texture_id,
                "width": texture.width,
                "height": texture.height,
                "layers": texture.layers,
                "kind": texture.kind,
                "format": "rgba8",
                "filter": texture.filter,
                "address": "clamp",
                "bytes": texture.pixels,
            }
        )
        self.texture_ids[key] = texture_id
        return texture_id

    def _pipeline(self, draw: PreparedDraw) -> int:
        key: PipelineKey = (
            draw.source,
            draw.state,
            tuple((texture.name, texture.kind) for texture in draw.textures),
        )
        if key in self.pipeline_ids:
            return self.pipeline_ids[key]
        pipeline_id = self._id()
        self.pipeline_ids[key] = pipeline_id
        self.pending_pipelines.append(
            {
                "id": pipeline_id,
                "source": draw.source,
                "state": {
                    "depth_test": draw.state.depth_test,
                    "depth_write": draw.state.depth_write,
                    "color_write": draw.state.color_write,
                    "stencil_compare": draw.state.stencil_compare,
                    "stencil_front": list(draw.state.stencil_front),
                    "stencil_back": list(draw.state.stencil_back),
                },
                "texture_names": [texture.name for texture in draw.textures],
                "texture_kinds": [texture.kind for texture in draw.textures],
            }
        )
        return pipeline_id
