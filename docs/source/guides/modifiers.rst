Modifiers, Keyframes, and Transitions
=====================================

Shrimply applies effects non-destructively as an ordered modifier stack on each
clip. Most numeric, vector, and color parameters can be animated with keyframes
or driven by :doc:`property expressions <expressions>`.

Keyframes and animation
-----------------------

Any keyframeable property in the inspector can vary over a clip's timeline:

* Add keyframes at the playhead to record values at specific frames, then open
  the keyframe graph to adjust timing and easing curves.
* To loop a clip's playback, open :menuselection:`Playback --> Repeat` in the
  inspector and set :guilabel:`Strategy` to :guilabel:`Repeat` or
  :guilabel:`Ping Pong`.
* For procedural or audio-reactive animation on a single property (such as
  oscillation, camera shake, or lip sync), enable an expression with the code
  button beside the property. See :doc:`expressions`.

Video modifiers
---------------

Video modifiers run in pipeline order from top to bottom on the selected visual
clip:

* **3D Scene modifiers**: configure 3D scene elements, lighting, materials,
  camera source (including :doc:`3D camera tracking <../server/camera-tracking>`),
  and ray-tracing denoising (``Auto`` or ``Off`` in the material inspector).
* **Vector stage**: vector shapes, text, SVG, and vectorized layers support
  vector modifiers before rasterization, including Transform, Repeat, Shaky
  Path, Path Offset, Opacity, HSV color adjustment, and Text Mask. Use
  **Vectorize** or **Rasterize** when crossing between vector and pixel stages.
* **Raster stage**: pixel effects include Transform, Crop, Corner Pin, Opacity,
  Chroma Key, Gaussian Blur, Directional Blur, Sharpen, Drop Shadow, Alpha
  Outline, Vignette, Glow/Bloom, Film Grain, Chromatic Aberration, Distortion
  effects (Fisheye, Lens Distortion, Twirl, Bulge/Pinch, Wave/Ripple, Mirror,
  Kaleidoscope), stylization (Kuwahara, Pixelate/Mosaic, Posterize, Threshold,
  Dithering, Halftone, Scanlines/CRT, Edge Detection, Emboss, Colorize/Duotone,
  Invert, Channel Mixer), **Segment Anything 2** masking
  (:doc:`../server/video-segmentation`), and **Cache** baking for heavy stacks.

Copy a clip's modifier stack and right-click another clip to **Paste
Modifiers**.

Audio modifiers
---------------

Audio clips support a non-destructive audio modifier stack, including:

* **Level & dynamics**: Gain, Pan, Stereo Width, Compressor, Limiter, Noise
  Gate.
* **Tone & repair**: Equalizer, Filter, Denoise, Voice Color, Close Up.
* **Time & creative**: Pitch, Echo, Reverb, Chorus, Tremolo, Distortion,
  Bitcrusher, **Voice Change** (:doc:`../server/voice-conversion`), and
  **Cache**.

Transitions
-----------

Clips support **Intro** and **Outro** transitions configured in the inspector
(such as fades in and out). Audio fades support **Equal Power** (default) and
**Linear** curves.
