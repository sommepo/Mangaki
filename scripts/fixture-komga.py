#!/usr/bin/env python3
"""Loopback-only authenticated Komga API fixture for Android acceptance tests."""

import argparse
import base64
import binascii
import json
import struct
import time
import zlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlsplit

USER = "fixture-user"
PASSWORD = "fixture-password"
AUTH = "Basic " + base64.b64encode(f"{USER}:{PASSWORD}".encode()).decode()

def png_chunk(kind, content):
    return struct.pack(">I", len(content)) + kind + content + struct.pack(">I", binascii.crc32(kind + content) & 0xFFFFFFFF)


def blank_png(width=320, height=240):
    rows = b"".join(b"\0" + b"\xff\xff\xff\xff" * width for _ in range(height))
    return b"\x89PNG\r\n\x1a\n" + png_chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)) + png_chunk(b"IDAT", zlib.compress(rows)) + png_chunk(b"IEND", b"")


PNG = blank_png()
PROGRESS = {}
PAGE_COUNT = 3
PAGE_DELAY = 0.0


def book(index):
    book_id = f"book-{index:04d}"
    return {
        "id": book_id,
        "seriesId": "series-unicode",
        "seriesTitle": "コミック",
        "name": f"Volume {index + 1}",
        "fileHash": f"fixture-hash-{index:04d}",
        "lastModified": "2026-09-08T12:00:00Z",
        "media": {"pagesCount": PAGE_COUNT, "status": "READY"},
        "readProgress": {"page": PROGRESS.get(book_id, 1)} if index == 0 else None,
    }


class Handler(BaseHTTPRequestHandler):
    server_version = "MangaReaderFixtureKomga/1"

    def log_message(self, _format, *_args):
        return

    def send_json(self, status, value):
        content = json.dumps(value).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(content)))
        self.end_headers()
        self.wfile.write(content)

    def authorised(self):
        if self.headers.get("Authorization") == AUTH:
            return True
        self.send_response(401)
        self.send_header("WWW-Authenticate", 'Basic realm="fixture"')
        self.end_headers()
        return False

    def do_GET(self):
        split = urlsplit(self.path)
        if split.path == "/fixture/status":
            self.send_json(200, {"progressWrites": len(PROGRESS), "book0Page": PROGRESS.get("book-0000")})
            return
        if not self.authorised():
            return
        if split.path == "/komga/api/v1/libraries":
            self.send_json(200, [{"id": "library-fixture", "name": "Fixture Library"}])
            return
        if split.path == "/komga/api/v1/books":
            query = parse_qs(split.query)
            if query.get("library_id") != ["library-fixture"]:
                self.send_json(400, {"error": "library_id required"})
                return
            page = int(query.get("page", ["0"])[0])
            start = page * 500
            self.send_json(200, {"content": [book(index) for index in range(start, min(start + 500, 501))]})
            return
        if split.path.startswith("/komga/api/v1/books/") and "/pages/" in split.path:
            if PAGE_DELAY:
                time.sleep(PAGE_DELAY)
            self.send_response(200)
            self.send_header("Content-Type", "image/png")
            self.send_header("Content-Length", str(len(PNG)))
            self.end_headers()
            self.wfile.write(PNG)
            return
        self.send_error(404)

    def do_PATCH(self):
        if not self.authorised():
            return
        split = urlsplit(self.path)
        prefix = "/komga/api/v1/books/"
        suffix = "/read-progress"
        if not split.path.startswith(prefix) or not split.path.endswith(suffix):
            self.send_error(404)
            return
        length = int(self.headers.get("Content-Length", "0"))
        payload = json.loads(self.rfile.read(length) or b"{}")
        book_id = split.path[len(prefix):-len(suffix)]
        PROGRESS[book_id] = int(payload["page"])
        self.send_response(204)
        self.end_headers()


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, default=28681)
    parser.add_argument("--pages", type=int, default=3)
    parser.add_argument("--delay", type=float, default=0.0)
    args = parser.parse_args()
    PAGE_COUNT = args.pages
    PAGE_DELAY = args.delay
    ThreadingHTTPServer(("127.0.0.1", args.port), Handler).serve_forever()
