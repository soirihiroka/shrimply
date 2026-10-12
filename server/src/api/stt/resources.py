from api.resource import ResourceProfile
from api.stt.protocol import ModelId

MIB = 1024**2
MAXIMUM_AUDIO_BYTES = 512 * MIB
MODELS = {
    "nvidia/parakeet-tdt-0.6b-v3": (627_057_286, 4, 512 * MIB),
    "Qwen/Qwen3-ASR-0.6B-hf": (782_426_112 + 917_728_896, 2, 768 * MIB),
    "openai/whisper-large-v3-turbo": (808_878_080, 2, 256 * MIB),
    "openai/whisper-small": (241_734_912, 4, 256 * MIB),
    "distil-whisper/distil-large-v3": (756_405_760, 2, 256 * MIB),
}


def request(model_id: ModelId, *, message_pack: bool = False) -> ResourceProfile:
    parameters, bytes_per_parameter, workspace = MODELS[model_id]
    return ResourceProfile(
        resident_vram=parameters * bytes_per_parameter,
        # MessagePack retains both the envelope and its audio while decoding;
        # concatenation briefly retains the chunks and the final sample buffer.
        active_ram=MAXIMUM_AUDIO_BYTES * (4 if message_pack else 2),
        active_vram=workspace,
    )
