import importlib
from collections.abc import Callable, Generator, Iterable
from contextlib import contextmanager
from typing import Protocol

from tqdm.auto import tqdm

report_progress: Callable[[str], None] | None = None
last_progress: str | None = None


class StreamedModelProgress(tqdm):
    def display(self, msg: str | None = None, pos: int | None = None) -> None:
        del msg, pos
        global last_progress
        if report_progress is None or not self.total:
            return
        percent = max(0, min(100, round(self.n * 100 / self.total)))
        description = self.desc.strip().rstrip(":")
        if self.unit == "B":
            description = (
                "Preparing model cache"
                if "reconstruct" in description.lower()
                else "Downloading model"
            )
        elif not description:
            description = "Loading model"
        message = f"{description} · {percent}%"
        if message != last_progress:
            last_progress = message
            report_progress(message)


type ProgressArgument = None | bool | int | float | str | Iterable[ProgressArgument]


class ProgressConstructor(Protocol):
    def __call__(
        self,
        *args: ProgressArgument,
        **kwargs: ProgressArgument,
    ) -> StreamedModelProgress: ...


def _transformers_progress(
    _factory: ProgressConstructor,
    args: tuple[ProgressArgument, ...],
    kwargs: dict[str, ProgressArgument],
) -> StreamedModelProgress:
    progress = StreamedModelProgress.__new__(StreamedModelProgress)
    initializer = getattr(progress, "__init__", None)
    if not callable(initializer):
        raise TypeError("tqdm progress implementation has no initializer")
    initializer(*args, **kwargs)
    return progress


@contextmanager
def stream_model_progress(
    report: Callable[[str], None], *, include_diffusers: bool = False
) -> Generator[None, None, None]:
    global last_progress, report_progress
    previous_report, previous_progress = report_progress, last_progress
    report_progress, last_progress = report, None

    transformers_logging = importlib.import_module("transformers.utils.logging")
    previous_transformers_hook = transformers_logging.set_tqdm_hook(
        _transformers_progress
    )
    snapshot_download = importlib.import_module("huggingface_hub._snapshot_download")
    previous_snapshot_tqdm = getattr(snapshot_download, "hf_tqdm")
    setattr(snapshot_download, "hf_tqdm", StreamedModelProgress)
    diffusers_logging = None
    previous_diffusers_tqdm = None
    if include_diffusers:
        diffusers_logging = importlib.import_module("diffusers.utils.logging")
        previous_diffusers_tqdm = diffusers_logging.tqdm_lib.tqdm
        diffusers_logging.tqdm_lib.tqdm = StreamedModelProgress
    try:
        yield
    finally:
        if diffusers_logging is not None:
            diffusers_logging.tqdm_lib.tqdm = previous_diffusers_tqdm
        setattr(snapshot_download, "hf_tqdm", previous_snapshot_tqdm)
        transformers_logging.set_tqdm_hook(previous_transformers_hook)
        report_progress, last_progress = previous_report, previous_progress
