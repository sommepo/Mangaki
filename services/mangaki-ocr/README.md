# Mangaki self-hosted OCR adapter

This optional service runs owoocr's segmented Manga OCR engine behind a small
HTTP interface. It combines comic-text-detector geometry with Manga OCR
transcription. Images are processed in memory and are neither stored nor
included in logs.

The service is a separate process and is not bundled into the Android APK.
owoocr and comic-text-detector are GPL-3.0-only; Manga OCR is Apache-2.0. The
adapter in this directory is therefore distributed under GPL-3.0-only. See
`LICENSE.md` and the pinned upstream versions in `requirements.lock`.
The installer applies the adjacent, versioned comic-text-detector patch which
lowers its text-line acceptance threshold from 0.60 to 0.45. This keeps
readable lower-confidence manga columns available to Manga OCR instead of
discarding them during geometry detection.

Run `scripts/install-mangaki-ocr-server.sh [bind-address]` from the repository
root. The installer creates an ignored virtual environment under `.toolchains`
and enables a user-level systemd service. It binds to `127.0.0.1` by default.
For Mangaki on another Tailnet device, pass this host's `tailscale0` IPv4
address; the installer rejects public and LAN bind addresses. The first start
downloads/loads the pinned models and may take roughly two minutes. Later warm
requests on the reference CPU take a few seconds per page.

The service limits PyTorch and systemd CPU use to three logical cores, bounds
the pending image queue, and applies 4/6 GiB soft/hard memory limits. Mangaki
also serializes background jobs and live self-hosted requests. These limits
keep the host responsive during whole-volume preparation; excess requests are
asked to retry instead of creating an unbounded number of waiting threads.

API:

- `GET /health` returns the engine/model identity.
- `POST /v1/ocr` accepts raw image bytes (maximum 64 MiB) and returns Mangaki's
  normalised OCR blocks.

Do not expose this unauthenticated service to a public interface. Use a
Tailscale ACL limiting TCP port 7332 to the two intended users/devices.

In Mangaki, open Settings → Manga reader, choose **Self-hosted Manga OCR**, enter
`http://<tailscale-address>:7332`, and use **Test OCR server**. Existing prepared
OCR is provider-specific; choose **Reprocess OCR** for a volume to replace it
with results from the newly selected provider.
