#!/usr/bin/env python3
"""Private HTTP adapter for owoocr's segmented Manga OCR engine.

This process deliberately returns only recognised text and normalised geometry.
It never stores images or logs recognised text.
"""

from __future__ import annotations

import json
import os
import re
import threading
import time
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any

from owocr.ocr import MangaOcrSegmented
import torch


ENGINE_NAME = "owoocr segmented Manga OCR"
MODEL_VERSION = "owocr-1.26.8+manga-ocr-0.1.16+comic-text-detector-line-0.45"
MAX_IMAGE_BYTES = 64 * 1024 * 1024
MAX_PENDING_REQUESTS = 3
JAPANESE_RE = re.compile(r"[\u3040-\u30ff\u3400-\u9fff]")
KANJI_RE = re.compile(r"[\u3400-\u9fff]")
KANA_RE = re.compile(r"[\u3040-\u30ff]")


def _line_text(line: Any) -> str:
    if line.text is not None:
        return str(line.text).strip()
    return "".join(str(word.text) for word in line.words).strip()


def _normalised_box(box: Any) -> dict[str, float]:
    return {
        "x": max(0.0, min(1.0, float(box.left))),
        "y": max(0.0, min(1.0, float(box.top))),
        "width": max(0.0, min(1.0, float(box.width))),
        "height": max(0.0, min(1.0, float(box.height))),
    }


def _intersection_ratio(inner: dict[str, Any], outer: dict[str, Any]) -> float:
    first = inner["tightBoundingBox"]
    second = outer["tightBoundingBox"]
    left = max(first["x"], second["x"])
    top = max(first["y"], second["y"])
    right = min(first["x"] + first["width"], second["x"] + second["width"])
    bottom = min(first["y"] + first["height"], second["y"] + second["height"])
    intersection = max(0.0, right - left) * max(0.0, bottom - top)
    area = first["width"] * first["height"]
    return intersection / area if area > 0.0 else 0.0


def _remove_nested_furigana(blocks: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """Suppress detector regions that are ruby embedded in a larger text block.

    Manga OCR already transcribes the base line. comic-text-detector can also
    return the adjacent, much smaller kana as a second overlapping paragraph.
    Requiring strong geometric containment makes this deliberately narrower
    than discarding small or kana-only speech bubbles in general.
    """
    kept: list[dict[str, Any]] = []
    for candidate in blocks:
        text = candidate["text"]
        bounds = candidate["tightBoundingBox"]
        candidate_area = bounds["width"] * bounds["height"]
        kana_only = bool(KANA_RE.search(text)) and not KANJI_RE.search(text)
        nested = kana_only and any(
            other is not candidate
            and other.get("forcedOrientation") == candidate.get("forcedOrientation") == "vertical"
            and candidate_area < (
                other["tightBoundingBox"]["width"] * other["tightBoundingBox"]["height"]
            ) * 0.45
            and bounds["width"] < other["tightBoundingBox"]["width"] * 0.6
            and _intersection_ratio(candidate, other) >= 0.65
            for other in blocks
        )
        if not nested:
            kept.append(candidate)
    return kept


def _to_blocks(result: Any) -> list[dict[str, Any]]:
    blocks: list[dict[str, Any]] = []
    for paragraph in result.paragraphs:
        vertical = paragraph.writing_direction == "TOP_TO_BOTTOM"
        lines = [(_line_text(line), line.bounding_box) for line in paragraph.lines]
        lines = [(text, box) for text, box in lines if text and JAPANESE_RE.search(text)]
        if not lines:
            continue

        # comic-text-detector emits vertical columns in reading order, but use
        # geometry as a deterministic fallback for model/library revisions.
        lines.sort(key=lambda item: item[1].center_x, reverse=vertical)
        separator = "\u200b" if vertical else ""
        text = separator.join(item[0].replace("\n", "") for item in lines)
        box = _normalised_box(paragraph.bounding_box)
        if not text or box["width"] <= 0.0 or box["height"] <= 0.0:
            continue
        blocks.append(
            {
                "text": text,
                "tightBoundingBox": box,
                "isMerged": True,
                "forcedOrientation": "vertical" if vertical else "horizontal",
            }
        )
    return _remove_nested_furigana(blocks)


class OcrRuntime:
    def __init__(self) -> None:
        thread_count = max(1, min(4, int(os.environ.get("MANGAKI_OCR_THREADS", "3"))))
        torch.set_num_threads(thread_count)
        try:
            torch.set_num_interop_threads(1)
        except RuntimeError:
            pass
        self._lock = threading.Lock()
        self._request_slots = threading.BoundedSemaphore(MAX_PENDING_REQUESTS)
        self.engine = MangaOcrSegmented({"force_cpu": True})
        if not self.engine.available:
            raise RuntimeError("owoocr segmented Manga OCR failed to initialise")

    def enter_queue(self) -> bool:
        return self._request_slots.acquire(blocking=False)

    def leave_queue(self) -> None:
        self._request_slots.release()

    def recognise(self, image: bytes) -> tuple[list[dict[str, Any]], float]:
        started = time.monotonic()
        with self._lock:
            ok, result = self.engine(image)
        if not ok:
            raise RuntimeError(str(result))
        return _to_blocks(result), time.monotonic() - started


class OcrHandler(BaseHTTPRequestHandler):
    runtime: OcrRuntime
    server_version = "MangakiOcr/1"

    def log_message(self, fmt: str, *args: Any) -> None:
        # Request metadata can identify a reader or page. Keep it out of logs;
        # the explicit lifecycle messages below contain only service health.
        return

    def _json(self, status: HTTPStatus, value: Any) -> None:
        body = json.dumps(value, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self) -> None:
        if self.path != "/health":
            self._json(HTTPStatus.NOT_FOUND, {"error": "not found"})
            return
        self._json(
            HTTPStatus.OK,
            {"status": "ok", "engine": ENGINE_NAME, "modelVersion": MODEL_VERSION},
        )

    def do_POST(self) -> None:
        if self.path != "/v1/ocr":
            self._json(HTTPStatus.NOT_FOUND, {"error": "not found"})
            return
        if not self.runtime.enter_queue():
            self.close_connection = True
            self._json(HTTPStatus.TOO_MANY_REQUESTS, {"error": "OCR queue is full; retry shortly"})
            return
        try:
            try:
                length = int(self.headers.get("Content-Length", "0"))
            except ValueError:
                length = 0
            if length <= 0 or length > MAX_IMAGE_BYTES:
                self._json(HTTPStatus.REQUEST_ENTITY_TOO_LARGE, {"error": "invalid image size"})
                return
            image = self.rfile.read(length)
            if len(image) != length:
                self._json(HTTPStatus.BAD_REQUEST, {"error": "incomplete image"})
                return
            try:
                blocks, elapsed = self.runtime.recognise(image)
            except Exception as error:
                print(f"mangaki-ocr recognition failed: {type(error).__name__}", flush=True)
                self._json(HTTPStatus.INTERNAL_SERVER_ERROR, {"error": "recognition failed"})
                return
            print(
                f"mangaki-ocr recognition complete blocks={len(blocks)} elapsed={elapsed:.3f}s",
                flush=True,
            )
            self._json(
                HTTPStatus.OK,
                {"engine": ENGINE_NAME, "modelVersion": MODEL_VERSION, "blocks": blocks},
            )
        finally:
            self.runtime.leave_queue()


def main() -> None:
    bind = os.environ.get("MANGAKI_OCR_BIND", "127.0.0.1")
    port = int(os.environ.get("MANGAKI_OCR_PORT", "7332"))
    OcrHandler.runtime = OcrRuntime()
    server = ThreadingHTTPServer((bind, port), OcrHandler)
    print(f"mangaki-ocr ready on http://{bind}:{port}", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
