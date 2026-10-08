Licenses
========

Shrimply is licensed under the GNU General Public License, version 3 or later.
The compute server is licensed separately under the GNU Affero General Public
License, version 3 or later.

The documentation layout and visual assets are copied from the GNOME Developer
Documentation site. Its templates are available under the MIT License and its
static theme assets under Creative Commons Attribution-ShareAlike 3.0. The
corresponding texts are available in the repository's `documentation licenses
<https://github.com/soirihiroka/shrimply/tree/main/docs/licenses>`__, and the
component is recorded in the `third-party notices
<https://github.com/soirihiroka/shrimply/blob/main/THIRDPARTY.md>`__.

Non-free and separately licensed components
--------------------------------------------

Shrimply and the compute server are free software, but some features depend on
components that are not free software or have additional use restrictions.
Those terms apply to the individual components and are not replaced by
Shrimply's GPL or AGPL licenses.

GPU features use NVIDIA's `CUDA Toolkit and display driver
<https://docs.nvidia.com/cuda/eula/>`__, `OptiX SDK
<https://developer.nvidia.com/designworks/optix/download>`__, `Optical Flow SDK
<https://developer.nvidia.com/optical-flow-sdk>`__, and `Video Codec SDK
<https://developer.nvidia.com/video-codec-sdk>`__ interfaces. Review NVIDIA's
terms before building or using those features. The NVIDIA Image Scaling code
included by Shrimply is separately available under the MIT License and is
listed in the repository's third-party notices.

The compute server can also download model weights governed by separate terms,
including MiniMax H3 and IndexTTS below. Additional model notices are listed in
the `server third-party notices
<https://github.com/soirihiroka/shrimply/blob/main/server/THIRDPARTY.md>`__.

.. _minimax-h3-license:

MiniMax H3
----------

The MiniMax H3 model weights are under the **MiniMax H3 Community License
Agreement**. Use is limited to its Applicable Territory, which excludes the
European Union, the United Kingdom, South Korea, and the United States. The
agreement has further limits on use, distribution, and commercial use. Read it
before downloading the model.

See the `upstream MiniMax H3 license
<https://huggingface.co/MiniMaxAI/MiniMax-H3/blob/main/LICENSE>`__.

.. _indextts-license:

IndexTTS
--------

IndexTTS 2 and 2.5 are under the **bilibili Model Use License Agreement**.
Read it before downloading the models. Shrimply keeps a copy with its IndexTTS
runtime and lists the models in the server's third-party notices.

See the `upstream IndexTTS license
<https://github.com/index-tts/index-tts/blob/main/LICENSE>`__, Shrimply's
`included copy
<https://github.com/soirihiroka/shrimply/blob/main/server/src/api/tts/index_tts_2_0/LICENSE>`__,
and the `server third-party notices
<https://github.com/soirihiroka/shrimply/blob/main/server/THIRDPARTY.md>`__.

See the repository's `main license
<https://github.com/soirihiroka/shrimply/blob/main/LICENSE>`__, `server license
<https://github.com/soirihiroka/shrimply/blob/main/server/LICENSE>`__, and
`third-party notices
<https://github.com/soirihiroka/shrimply/blob/main/THIRDPARTY.md>`__ for the
complete terms.
