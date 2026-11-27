# T3Box physical acceptance gates

Docker/QEMU can run Linux AArch64 userspace, but cannot simulate Raspberry Pi 5
VideoCore, RP1 I/O controller, GPIO, the selected HDMI UVC capture firmware,
USB sound adapter, projector EDID, Anker PD negotiation, or Wi-Fi radio.

Before a real LT event:

- [ ] Verify 1080p30 capture with presentation slides and small code font
- [ ] Check HDMI passthrough while the Raspberry Pi restarts / loses power
- [ ] Verify projector EDID and multiple presenters' Windows/Mac adapters
- [ ] Confirm audio captures at line level without clipping or noise
- [ ] Measure A/V sync drift during a 3-hour capture session
- [ ] Verify HDMI disconnect/reconnect, and reconnection after USB replug
- [ ] Verify YouTube Studio actually receives RTMPS and both audio/video
- [ ] Test upstream Wi-Fi speed under venue load and packet loss
- [ ] Test Anker battery model and powered USB current/thermal headroom
- [ ] Test power failure, recovery, and shutdown behavior
- [ ] Test GPIO input/output and local Wi-Fi setup UX in the real venue
- [ ] Run workflow .github/workflows/hardware.yml on a self-hosted Pi 5

No boxes are checked because physical testing has not occurred.

Detailed procedures, pass criteria and the results log (guarded in CI by
`ci/check-qualification.py`) are in docs/QUALIFICATION.md.
