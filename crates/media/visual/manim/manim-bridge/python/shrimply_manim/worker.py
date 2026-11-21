from __future__ import annotations

import importlib.util
import inspect
import sys
from collections.abc import Callable
from fractions import Fraction
from pathlib import Path

from shrimply_manim.worker_types import Message, ParameterOverrides


sys.path.insert(0, str(Path(__file__).parent.parent))

print("Shrimply Manim worker: importing Manim", file=sys.stderr, flush=True)
import numpy as np
from numpy.typing import NDArray
from shrimply_manim.render_pool import (
    PreparedFrame,
    install_capture,
    prepare_frame,
)
from shrimply_manim import parameters as reflected_parameters

install_capture()

from manimlib.animation.animation import Animation, prepare_animation
from manimlib import manim_config
from manimlib.mobject.mobject import _AnimationBuilder
from manimlib.scene.scene import Scene
print("Shrimply Manim worker: Manim imported", file=sys.stderr, flush=True)

type FrameCallback = Callable[[int, Fraction, PreparedFrame], None]


def exact(value: int | float | Fraction) -> Fraction:
    return Fraction(str(value))


def _new_scene(
    source: Path,
    requested_name: str,
    width: int,
    height: int,
    fps: Fraction,
    module_name: str,
) -> tuple[Scene, list[str], str]:
    manim_config.camera.resolution = (width, height)
    sys.path.insert(0, str(source.parent))
    spec = importlib.util.spec_from_file_location(module_name, source)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"could not load {source}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    scene_classes = [
        value
        for _, value in inspect.getmembers(module, inspect.isclass)
        if issubclass(value, Scene) and value is not Scene and value.__module__ == module.__name__
    ]
    if not scene_classes:
        raise RuntimeError(f"{source} defines no Manim Scene")
    names = [scene.__name__ for scene in scene_classes]
    if requested_name:
        selected = next((scene for scene in scene_classes if scene.__name__ == requested_name), None)
        if selected is None:
            raise RuntimeError(f"scene {requested_name!r} was not found; available scenes: {', '.join(names)}")
    else:
        selected = scene_classes[0]
    scene = selected(
        camera_config={
            "resolution": (width, height),
            "fps": float(fps),
            "background_opacity": 0.0,
        },
        file_writer_config={
            "write_to_movie": False,
            "save_last_frame": False,
            "quiet": True,
        },
        skip_animations=True,
    )
    return scene, names, selected.__name__


def load_scene(
    source: Path,
    requested_name: str,
    width: int,
    height: int,
    fps: Fraction,
    on_frame: FrameCallback,
    parameter_overrides: ParameterOverrides,
) -> tuple[list[str], str, list[Message], bool, int]:
    print(f"Shrimply Manim render pass: loading {source}", file=sys.stderr, flush=True)
    reflected_parameters.begin(parameter_overrides)
    try:
        scene, names, selected_name = _new_scene(
            source,
            requested_name,
            width,
            height,
            fps,
            "shrimply_user_scene_render",
        )
    except BaseException:
        reflected_parameters.cancel()
        raise
    print(
        f"Shrimply Manim render pass: constructing scene {selected_name}",
        file=sys.stderr,
        flush=True,
    )
    next_frame_index = 0

    def emit_frame(self: Scene, position: Fraction) -> None:
        nonlocal next_frame_index
        on_frame(next_frame_index, position, prepare_frame(self))
        next_frame_index += 1

    def advance_animations(
        self: Scene,
        animations: list[Animation],
        elapsed: np.float64,
        delta: np.float64,
    ) -> None:
        for animation in animations:
            animation.update_reference_mobjects(delta, frame_rate=float(fps))
            animation.interpolate(
                1.0 if animation.run_time <= 0 else elapsed / animation.run_time
            )
        self.increment_time(delta)
        self.update_mobjects(delta)

    def frame_times(duration: Fraction) -> NDArray[np.float64]:
        step = 1 / float(fps)
        return np.arange(0, float(duration), step) + step

    def finish_animations(self: Scene, animations: list[Animation]) -> None:
        for animation in animations:
            animation.finish()
            animation.clean_up_from_scene(self)
        self.update_mobjects(0)

    def record_play(
        self: Scene,
        *proto_animations: Animation | _AnimationBuilder,
        run_time: int | float | Fraction | None = None,
        rate_func: Callable[[float], float] | None = None,
        lag_ratio: float | None = None,
    ) -> None:
        nonlocal next_frame_index
        animations = [prepare_animation(animation) for animation in proto_animations]
        for animation in animations:
            animation.update_rate_info(run_time, rate_func, lag_ratio)
        duration = exact(self.get_run_time(animations))
        self.pre_play()
        self.begin_animations(animations)
        previous = np.float64(0)
        for elapsed in frame_times(duration):
            position = Fraction(next_frame_index, 1) / fps
            advance_animations(self, animations, elapsed, elapsed - previous)
            previous = elapsed
            emit_frame(self, position)
        finish_animations(self, animations)
        self.post_play()

    def record_wait(
        self: Scene,
        duration: int | float | Fraction | None = None,
        stop_condition: Callable[[], bool] | None = None,
        note: str | None = None,
        ignore_presenter_mode: bool = False,
    ) -> None:
        nonlocal next_frame_index
        if duration is None:
            duration = self.default_wait_time
        duration = exact(duration)
        self.pre_play()
        self.update_mobjects(0)
        previous = np.float64(0)
        for elapsed in frame_times(duration):
            position = Fraction(next_frame_index, 1) / fps
            delta = elapsed - previous
            self.increment_time(delta)
            self.update_mobjects(delta)
            previous = elapsed
            emit_frame(self, position)
            if stop_condition is not None and stop_condition():
                break
        self.post_play()

    scene.play = record_play.__get__(scene)
    scene.wait = record_wait.__get__(scene)
    try:
        scene.setup()
        scene.construct()
        if next_frame_index == 0:
            emit_frame(scene, Fraction(0))
    except BaseException:
        reflected_parameters.cancel()
        raise
    scene.camera.samples = reflected_parameters.reflect_anti_aliasing(scene.camera.samples)
    parameters, render_is_current = reflected_parameters.finish()
    print(
        f"Shrimply Manim render pass: scene ready at {scene.time:.2f}s with {next_frame_index} emitted frames",
        file=sys.stderr,
        flush=True,
    )
    return names, selected_name, parameters, render_is_current, scene.camera.samples
