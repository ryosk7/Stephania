# Experimental dual audio

Use HDMI embedded audio and a USB microphone without a hardware mixer. The default single-input mode is unchanged.

```sh
arecord -l
arecord -L
export T3BOX_AUDIO_MODE=mix
export T3BOX_AUDIO_HDMI=plughw:CARD=Capture,DEV=0
export T3BOX_AUDIO_MIC=plughw:CARD=Microphone,DEV=0
export T3BOX_AUDIO_MIC_CHANNELS=1
export T3BOX_AUDIO_HDMI_GAIN_DB=-12
export T3BOX_AUDIO_MIC_GAIN_DB=-6
export T3BOX_AUDIO_LATENCY_MS=40
```

Replace example ALSA identifiers with actual devices. Use a stereo HDMI input and a mono or stereo microphone. ALSA plug conversion supplies 48 kHz S16 PCM. Gain range is -60 to +6 dB; latency range is 20–100 ms. Both inputs use independent existing AudioClock correction and bounded 8192-frame queues. AAC remains 48 kHz stereo, with continuous 1024-frame PTS and block peak limiting. Missing input is replaced with silence; capture errors reset that input. Network/encoder stalls exceeding the mixer deadline fail the session rather than replaying arbitrarily stale audio.

This is an initial implementation: timestamp estimation still uses capture arrival time, gain changes require restart, there is no runtime meter/API, adaptive buffer feedback, gain smoothing or calibrated offsets. Existing ALSA retry and capture lifecycle are unchanged. Virtual capture currently exercises the legacy path; mixer unit tests exercise mono/stereo, silence, clipping and deadline. Before production, validate independent USB clock drift, hotplug and stop/restart, end-to-end AAC decoding, both publisher paths, A/V sync, CPU and USB power on real Pi 5. No measured performance claim is made.
