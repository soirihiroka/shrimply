Video Generation
================

Generate video clips from text prompts and optional reference images or audio.

Create a generated video
------------------------

Add a video-generation item from the add menu on a video track. Choose a
model, fill in the text and media it asks for, and start generation.

Leave the server running until it finishes. Cancel from Shrimply if you don't
need the result.

Available models
----------------

``MiniMax H3 Base``
   Text-to-video, first and last frame to video, and reference images in order.

``Looping Sketch Anime``
   Text-to-video, and first and last frame to video.

``Wan 2.1 T2V 1.3B``
   Text-to-video, landscape or portrait.

``Wan 2.2 TI2V 5B``
   Text-to-video and a first-frame image, landscape or portrait.

Requirements, formats, and limits
---------------------------------

* **CUDA required**: Video Generation requires an NVIDIA CUDA device on the
  compute server and is hidden when the server runs on CPU.
* **Memory**: These models are large. ``Low VRAM`` moves work into system
  memory and is slower. Quantized models use less memory and can look worse.
* **Reference images**: accepts raster formats (JPEG, PNG, WebP, TIFF, BMP,
  GIF, AVIF, JPEG 2000, QOI, ICO, PPM, TGA). SVG, PDF, and layered formats
  (PSD) are not accepted. Images are oriented using EXIF metadata, transparent
  pixels are composited onto white, and animated or multi-page images use their
  first frame.
* **Upload size limits**: up to **256 MiB** per media item and **512 MiB**
  combined media per request.

Licenses and model pages
------------------------

MiniMax H3 is under the :ref:`MiniMax H3 Community License Agreement <minimax-h3-license>`.
Read it before downloading. Its territory limit excludes the European Union,
the United Kingdom, South Korea, and the United States.

Wan model pages: `Wan 2.1 1.3B <https://huggingface.co/Wan-AI/Wan2.1-T2V-1.3B-Diffusers>`__
and `Wan 2.2 5B <https://huggingface.co/Wan-AI/Wan2.2-TI2V-5B-Diffusers>`__.
