import math
from collections.abc import Sequence
from typing import Literal

import torch
from api.pneuma.inference.vendor.bigvgan.bigvgan import AMPBlock1, AMPBlock2
from api.pneuma.inference.vendor.bigvgan.env import AttrDict
from torch import nn
from torch.nn import Conv1d, ConvTranspose1d
from torch.nn import functional as F
from torch.nn.utils.parametrizations import weight_norm

from api.pneuma.inference.models.generators import SourceModuleHnNSF
from api.pneuma.inference.models.utils import init_weights, remove_weight_norm


class _BigVGANNSFBase(nn.Module):
    def __init__(
        self,
        initial_channel: int,
        resblock: Literal["1", "2"],
        resblock_kernel_sizes: Sequence[int],
        resblock_dilation_sizes: Sequence[Sequence[int]],
        upsample_rates: Sequence[int],
        upsample_initial_channel: int,
        upsample_kernel_sizes: Sequence[int],
        sr: Literal["48k", "48000"] | int,
        is_half: bool = False,
    ) -> None:
        super().__init__()
        sr_value = 48000 if sr in {"48k", "48000"} else sr
        if sr_value != 48000:
            raise ValueError("v3 BigVGAN-NSF generator currently supports only 48k.")

        self.num_kernels = len(resblock_kernel_sizes)
        self.num_upsamples = len(upsample_rates)
        self.upp = math.prod(upsample_rates)
        self.m_source = SourceModuleHnNSF(
            sampling_rate=sr_value,
            harmonic_num=0,
            is_half=is_half,
        )
        self.conv_pre = weight_norm(
            Conv1d(initial_channel, upsample_initial_channel, 7, 1, padding=3)
        )
        self.noise_convs = nn.ModuleList()
        self.ups = nn.ModuleList()
        self.resblocks = nn.ModuleList()
        block_hparams = AttrDict(
            {
                "snake_logscale": True,
                "use_cuda_kernel": False,
            }
        )
        if not upsample_rates or not upsample_kernel_sizes:
            raise ValueError("V3 BigVGAN-NSF generator requires at least one upsample.")
        ch = upsample_initial_channel
        for i, (u, k) in enumerate(zip(upsample_rates, upsample_kernel_sizes)):
            ch = upsample_initial_channel // (2 ** (i + 1))
            self.ups.append(
                weight_norm(
                    ConvTranspose1d(
                        upsample_initial_channel // (2**i),
                        ch,
                        k,
                        u,
                        padding=(k - u) // 2,
                    )
                )
            )
            if i + 1 < len(upsample_rates):
                stride_f0 = math.prod(upsample_rates[i + 1 :])
                self.noise_convs.append(
                    Conv1d(
                        1,
                        ch,
                        kernel_size=stride_f0 * 2,
                        stride=stride_f0,
                        padding=stride_f0 // 2,
                    )
                )
            else:
                self.noise_convs.append(Conv1d(1, ch, kernel_size=1))
            for k_res, dilation in zip(resblock_kernel_sizes, resblock_dilation_sizes):
                if resblock == "1":
                    if len(dilation) != 3:
                        raise ValueError(
                            "AMPBlock1 requires exactly three dilation rates."
                        )
                    first_dilation, second_dilation, third_dilation = dilation
                    self.resblocks.append(
                        AMPBlock1(
                            block_hparams,
                            ch,
                            k_res,
                            (first_dilation, second_dilation, third_dilation),
                            activation="snakebeta",
                        )
                    )
                else:
                    self.resblocks.append(
                        AMPBlock2(
                            block_hparams,
                            ch,
                            k_res,
                            tuple(dilation),
                            activation="snakebeta",
                        )
                    )

        self.activation_post = self._make_activation(ch)
        self.conv_post = weight_norm(Conv1d(ch, 1, 7, 1, padding=3, bias=False))
        self.ups.apply(init_weights)
        self.conv_post.apply(init_weights)

    @staticmethod
    def _make_activation(channels: int) -> nn.Module:
        from api.pneuma.inference.vendor.bigvgan import activations
        from api.pneuma.inference.vendor.bigvgan.alias_free_activation.torch.act import Activation1d

        return Activation1d(
            activation=activations.SnakeBeta(channels, alpha_logscale=True)
        )

    @staticmethod
    def _check_forward_inputs(
        x: torch.Tensor,
        f0: torch.Tensor,
        g: torch.Tensor | None,
        n_res: torch.Tensor | None,
    ) -> None:
        if x.dim() != 3:
            raise ValueError(
                f"Expected x to be [batch, channels, frames], got {x.shape}"
            )
        if f0.dim() != 2:
            raise ValueError(f"Expected f0 to be [batch, frames], got {f0.shape}")
        if x.shape[0] != f0.shape[0]:
            raise ValueError(
                f"Expected x and f0 batch sizes to match, got {x.shape[0]} and {f0.shape[0]}"
            )
        if n_res is None and x.shape[-1] != f0.shape[1]:
            raise ValueError(
                "Expected x and f0 frame counts to match when n_res is not set, "
                f"got x={x.shape}, f0={f0.shape}"
            )
        if g is not None:
            if g.dim() != 3:
                raise ValueError(
                    f"Expected g to be [batch, gin_channels, 1], got {g.shape}"
                )
            if g.shape[0] != x.shape[0] or g.shape[2] != 1:
                raise ValueError(
                    "Expected g to share the x batch size and have length 1, "
                    f"got x={x.shape}, g={g.shape}"
                )
        if n_res is not None and n_res.dim() != 0:
            raise ValueError(f"Expected n_res to be a scalar tensor, got {n_res.shape}")

    def forward(
        self,
        x: torch.Tensor,
        f0: torch.Tensor,
        global_conditioning: torch.Tensor | None = None,
        n_res: torch.Tensor | None = None,
    ) -> torch.Tensor:
        return self._forward(x, f0, global_conditioning, n_res)

    def _forward(
        self,
        x: torch.Tensor,
        f0: torch.Tensor,
        global_conditioning: torch.Tensor | None,
        n_res: torch.Tensor | None,
    ) -> torch.Tensor:
        self._check_forward_inputs(x, f0, global_conditioning, n_res)
        har_source, _noi_source, _uv = self.m_source(f0, self.upp)
        har_source = har_source.transpose(1, 2)
        if n_res is not None:
            n = int(n_res.item())
            if n * self.upp != har_source.shape[-1]:
                har_source = F.interpolate(har_source, size=n * self.upp, mode="linear")
            if n != x.shape[-1]:
                x = F.interpolate(x, size=n, mode="linear")

        x = self.conv_pre(x)
        x = self._apply_conditioning(x, global_conditioning)

        for i, (upsample, noise_conv) in enumerate(zip(self.ups, self.noise_convs)):
            x = upsample(x)
            x = x + noise_conv(har_source)
            xs: torch.Tensor | None = None
            for j, resblock in enumerate(self.resblocks):
                if i * self.num_kernels <= j < (i + 1) * self.num_kernels:
                    xs = resblock(x) if xs is None else xs + resblock(x)
            if xs is None:
                raise ValueError("No BigVGAN AMP blocks were configured.")
            x = xs / self.num_kernels

        x = self.activation_post(x)
        x = self.conv_post(x)
        return torch.clamp(x, min=-1.0, max=1.0)

    def _apply_conditioning(
        self, x: torch.Tensor, global_conditioning: torch.Tensor | None
    ) -> torch.Tensor:
        if global_conditioning is not None:
            raise ValueError("Speakerless BigVGAN does not accept global conditioning.")
        return x

    def frames_for_samples(self, sample_count: int) -> int:
        if sample_count <= 0:
            raise ValueError(f"sample_count must be positive, got {sample_count}.")
        if sample_count % self.upp != 0:
            raise ValueError(
                f"sample_count {sample_count} is not divisible by decoder ratio {self.upp}."
            )
        return sample_count // self.upp

    def remove_weight_norm(self) -> None:
        remove_weight_norm(self.conv_pre)
        for upsample in self.ups:
            remove_weight_norm(upsample)
        for resblock in self.resblocks:
            if not isinstance(resblock, (AMPBlock1, AMPBlock2)):
                raise TypeError("BigVGAN has an invalid residual block")
            resblock.remove_weight_norm()
        remove_weight_norm(self.conv_post)


class SpeakerConditionedBigVGANNSFGenerator(_BigVGANNSFBase):
    def __init__(
        self,
        initial_channel: int,
        resblock: Literal["1", "2"],
        resblock_kernel_sizes: Sequence[int],
        resblock_dilation_sizes: Sequence[Sequence[int]],
        upsample_rates: Sequence[int],
        upsample_initial_channel: int,
        upsample_kernel_sizes: Sequence[int],
        gin_channels: int,
        sr: Literal["48k", "48000"] | int,
        is_half: bool = False,
    ) -> None:
        super().__init__(
            initial_channel,
            resblock,
            resblock_kernel_sizes,
            resblock_dilation_sizes,
            upsample_rates,
            upsample_initial_channel,
            upsample_kernel_sizes,
            sr=sr,
            is_half=is_half,
        )
        self.cond = Conv1d(gin_channels, upsample_initial_channel, 1)

    def forward(
        self,
        x: torch.Tensor,
        f0: torch.Tensor,
        global_conditioning: torch.Tensor | None = None,
        n_res: torch.Tensor | None = None,
    ) -> torch.Tensor:
        return self._forward(x, f0, global_conditioning, n_res)

    def _apply_conditioning(
        self, x: torch.Tensor, global_conditioning: torch.Tensor | None
    ) -> torch.Tensor:
        if global_conditioning is None:
            raise ValueError(
                "Speaker-conditioned BigVGAN requires speaker conditioning."
            )
        return x + self.cond(global_conditioning)
