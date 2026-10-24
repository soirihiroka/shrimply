from typing import Protocol

import torch
from torch import nn
from transformers import GPT2Config, GPT2Model, GPT2PreTrainedModel
from transformers.cache_utils import Cache
from transformers.generation import GenerationMixin
from transformers.modeling_outputs import CausalLMOutputWithCrossAttentions

type GenerationArgument = torch.Tensor | Cache | bool | int | None


class PositionEmbedding(Protocol):
    def __call__(self, inputs: torch.Tensor) -> torch.Tensor: ...

    def get_fixed_embedding(
        self, index: int, device: torch.device
    ) -> torch.Tensor: ...


class NullPositionEmbeddings(nn.Module):
    zero: torch.Tensor

    def __init__(self, dimension: int) -> None:
        super().__init__()
        self.dimension = dimension
        self.register_buffer("zero", torch.tensor(0.0), persistent=False)

    def forward(self, position_ids: torch.Tensor) -> torch.Tensor:
        return torch.zeros(
            (*position_ids.shape, self.dimension),
            device=position_ids.device,
            dtype=self.zero.dtype,
        )


class GPT2InferenceModel(GPT2PreTrainedModel, GenerationMixin):
    def __init__(
        self,
        config: GPT2Config,
        gpt: GPT2Model,
        text_pos_emb: PositionEmbedding,
        embeddings: nn.Module,
        norm: nn.Module,
        linear: nn.Module,
        kv_cache: bool = False,
    ) -> None:
        super().__init__(config)
        self.transformer = gpt
        self.text_pos_embedding = text_pos_emb
        self.embeddings = embeddings
        self.final_norm = norm
        self.lm_head = nn.Sequential(norm, linear)
        self.kv_cache = kv_cache
        self.cached_mel_emb: torch.Tensor | None = None

    def get_output_embeddings(self) -> nn.Module:
        return self.lm_head

    def set_output_embeddings(self, new_embeddings: nn.Module) -> None:
        self.lm_head = new_embeddings

    def store_mel_emb(self, mel_emb: torch.Tensor) -> None:
        self.cached_mel_emb = mel_emb

    def prepare_inputs_for_generation(
        self,
        input_ids: torch.Tensor,
        next_sequence_length: int | None = None,
        past_key_values: Cache | None = None,
        attention_mask: torch.Tensor | None = None,
        inputs_embeds: torch.Tensor | None = None,
        is_first_iteration: bool | None = False,
        **kwargs: GenerationArgument,
    ) -> dict[str, GenerationArgument]:
        del inputs_embeds, is_first_iteration
        token_type_argument = kwargs.get("token_type_ids")
        token_type_ids = (
            token_type_argument
            if isinstance(token_type_argument, torch.Tensor)
            else None
        )
        position_argument = kwargs.get("position_ids")
        position_ids = (
            position_argument if isinstance(position_argument, torch.Tensor) else None
        )
        use_cache_argument = kwargs.get("use_cache")
        use_cache = use_cache_argument if isinstance(use_cache_argument, bool) else None

        cache_length = (
            past_key_values.get_seq_length()
            if self.kv_cache and past_key_values is not None
            else 0
        )
        if not self.kv_cache:
            past_key_values = None
        if cache_length:
            sequence_length = next_sequence_length or 1
            input_ids = input_ids[:, -sequence_length:].clone(
                memory_format=torch.contiguous_format
            )
            if token_type_ids is not None:
                token_type_ids = token_type_ids[:, -sequence_length:].clone(
                    memory_format=torch.contiguous_format
                )
        if attention_mask is not None and position_ids is None:
            position_ids = attention_mask.long().cumsum(-1) - 1
            position_ids.masked_fill_(attention_mask == 0, 0)
        if position_ids is not None and cache_length:
            position_ids = position_ids[:, -input_ids.shape[1] :]
        return {
            "input_ids": input_ids,
            "past_key_values": past_key_values,
            "use_cache": use_cache,
            "position_ids": position_ids,
            "attention_mask": attention_mask,
            "token_type_ids": token_type_ids,
        }

    def forward(
        self,
        input_ids: torch.Tensor | None = None,
        past_key_values: Cache | None = None,
        attention_mask: torch.Tensor | None = None,
        token_type_ids: torch.Tensor | None = None,
        position_ids: torch.Tensor | None = None,
        inputs_embeds: torch.Tensor | None = None,
        encoder_hidden_states: torch.Tensor | None = None,
        encoder_attention_mask: torch.Tensor | None = None,
        use_cache: bool | None = None,
        output_attentions: bool | None = None,
        output_hidden_states: bool | None = None,
        return_dict: bool | None = None,
        **kwargs: GenerationArgument,
    ) -> CausalLMOutputWithCrossAttentions:
        del inputs_embeds, output_attentions, output_hidden_states
        if return_dict is False:
            raise ValueError("IndexTTS 2 generation requires structured model output")
        if self.cached_mel_emb is None:
            raise RuntimeError("The GPT prefix must be stored before generation")
        if input_ids is None:
            raise ValueError("GPT input IDs are required")

        prefix_length = self.cached_mel_emb.shape[1]
        if input_ids.shape[1] != 1:
            audio_tokens = input_ids[:, prefix_length:]
            audio_embeddings = self.embeddings(audio_tokens)
            audio_embeddings += self.text_pos_embedding(audio_embeddings)
            prefix = self.cached_mel_emb
            if prefix.shape[0] != audio_embeddings.shape[0]:
                prefix = prefix.repeat_interleave(
                    audio_embeddings.shape[0] // prefix.shape[0], dim=0
                )
            embeddings = torch.cat((prefix, audio_embeddings), dim=1)
        else:
            if past_key_values is None:
                raise ValueError("Cached GPT generation requires a populated cache")
            # Transformers can omit an all-ones mask during generation. The
            # cache counts the prefix and previous audio tokens, not this token.
            audio_position = past_key_values.get_seq_length() - prefix_length
            if audio_position < 0:
                raise ValueError("Cached GPT generation is missing its prefix")
            embeddings = self.embeddings(input_ids)
            embeddings += self.text_pos_embedding.get_fixed_embedding(
                audio_position,
                input_ids.device,
            )

        transformer_output = self.transformer(
            inputs_embeds=embeddings,
            past_key_values=past_key_values,
            attention_mask=attention_mask,
            token_type_ids=token_type_ids,
            position_ids=position_ids,
            encoder_hidden_states=encoder_hidden_states,
            encoder_attention_mask=encoder_attention_mask,
            use_cache=use_cache,
            **kwargs,
        )
        logits = self.lm_head(transformer_output.last_hidden_state)
        return CausalLMOutputWithCrossAttentions(
            loss=None,
            logits=logits,
            past_key_values=transformer_output.past_key_values,
            hidden_states=transformer_output.hidden_states,
            attentions=transformer_output.attentions,
            cross_attentions=transformer_output.cross_attentions,
        )

    @staticmethod
    def _reorder_cache(
        past_key_values: Cache, beam_index: torch.LongTensor
    ) -> Cache:
        past_key_values.reorder_cache(beam_index)
        return past_key_values
