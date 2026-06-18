# trigger6

Reverse-engineering workspace for MCT Trigger 6 USB display adapters.

The current target is understanding the T6 protocol well enough to build a new
display driver path with rotation support. Mac is the first practical target
because the existing vendor driver lacks rotation support there, but the larger
goal is an all-platform implementation across Mac, Windows, and Linux.

The existing Linux DRM/KMS code and captures are useful reference material. In
the long run, Linux support should share the same T6 protocol knowledge rather
than becoming a separate reverse-engineering effort.

Current reverse-engineering notes:

- `docs/reverse-engineering-notes.md`
- `docs/linux-driver-gap-analysis.md`
- `docs/windows-capture-guide.md`

PCAP helper:

```sh
python3 tools/t6_pcap_summary.py captures/mctt6.pcapng --summary-only
```

One useful Windows-side milestone is an MP4 playback demo on a JUA365-attached
display: first as a USBPcap capture scenario, and later as a direct T6 transport
demo once the protocol is solid enough.
