# Rubber Band LiveShifter

Rust port of the single-window R3 LiveShifter in [Rubber Band 4.0.0](https://breakfastquay.com/files/releases/rubberband-4.0.0.tar.bz2). It includes the live pitch-shifting path used by Shrimply: 512-frame blocks, optional classification lookahead, cepstral formant correction, stereo channel locking, and the dynamic BQ resampler. FFTs use the workspace's Rust RealFFT dependency.

The upstream archive's SHA-256 is `af050313ee63bc18b35b2e064e5dce05b276aaf6d1aa2b8a82ced1fe2f8028e9`. See [COPYING](COPYING) for the license.
