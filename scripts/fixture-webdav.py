#!/usr/bin/env python3
"""Loopback-only authenticated WebDAV fixture for Android acceptance tests."""

import argparse
import base64
import binascii
import io
import struct
import zipfile
import zlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import quote, unquote, urlsplit

USER = "fixture-user"
PASSWORD = "fixture-password"
AUTH = "Basic " + base64.b64encode(f"{USER}:{PASSWORD}".encode()).decode()

def png_chunk(kind, content):
    return struct.pack(">I", len(content)) + kind + content + struct.pack(">I", binascii.crc32(kind + content) & 0xFFFFFFFF)


def blank_png(width=320, height=240):
    rows = b"".join(b"\0" + b"\xff\xff\xff\xff" * width for _ in range(height))
    return b"\x89PNG\r\n\x1a\n" + png_chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)) + png_chunk(b"IDAT", zlib.compress(rows)) + png_chunk(b"IEND", b"")


PNG = blank_png()


def archive_bytes():
    output = io.BytesIO()
    with zipfile.ZipFile(output, "w", zipfile.ZIP_DEFLATED) as archive:
        archive.writestr("010.png", PNG)
        archive.writestr("002.png", PNG)
    return output.getvalue()


ARCHIVE = archive_bytes()
FILES = {
    "/dav/シリーズ壱/Vol 2/001.png": PNG,
    "/dav/シリーズ壱/Vol 2/002.PNG": PNG,
    "/dav/シリーズ壱/Vol 10.cbz": ARCHIVE,
}
FOLDERS = {"/dav/", "/dav/シリーズ壱/", "/dav/シリーズ壱/Vol 2/"}


def encoded(path):
    return quote(path, safe="/")


class Handler(BaseHTTPRequestHandler):
    server_version = "MangaReaderFixtureWebDAV/1"

    def log_message(self, _format, *_args):
        print(f"{self.command} {urlsplit(self.path).path}", flush=True)

    def authorised(self):
        if self.headers.get("Authorization") == AUTH:
            return True
        self.send_response(401)
        self.send_header("WWW-Authenticate", 'Basic realm="fixture"')
        self.end_headers()
        return False

    def do_PROPFIND(self):
        if not self.authorised():
            return
        path = unquote(urlsplit(self.path).path)
        if not path.endswith("/") and path + "/" in FOLDERS:
            path += "/"
        if path not in FOLDERS and path not in FILES:
            self.send_error(404)
            return
        depth = self.headers.get("Depth", "0")
        members = [path]
        if depth != "0" and path in FOLDERS:
            members.extend(sorted(candidate for candidate in FOLDERS | set(FILES) if candidate != path and candidate.startswith(path) and "/" not in candidate[len(path):].rstrip("/")))
        responses = []
        for member in members:
            is_folder = member in FOLDERS
            size = 0 if is_folder else len(FILES[member])
            resource_type = "<d:collection/>" if is_folder else ""
            responses.append(
                f"<d:response><d:href>{encoded(member)}</d:href><d:propstat><d:prop>"
                f"<d:resourcetype>{resource_type}</d:resourcetype>"
                f"<d:getlastmodified>Tue, 08 Sep 2026 12:00:00 GMT</d:getlastmodified>"
                f"<d:getcontentlength>{size}</d:getcontentlength>"
                f"<d:getcontenttype>{'httpd/unix-directory' if is_folder else 'application/octet-stream'}</d:getcontenttype>"
                f"<d:getetag>\"fixture-{size}\"</d:getetag>"
                "</d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>"
            )
        body = ('<?xml version="1.0" encoding="utf-8"?><d:multistatus xmlns:d="DAV:">' + "".join(responses) + "</d:multistatus>").encode()
        self.send_response(207)
        self.send_header("Content-Type", "application/xml; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if not self.authorised():
            return
        path = unquote(urlsplit(self.path).path)
        content = FILES.get(path)
        if content is None:
            self.send_error(404)
            return
        self.send_response(200)
        self.send_header("Content-Type", "application/zip" if path.lower().endswith((".zip", ".cbz")) else "image/png")
        self.send_header("Content-Length", str(len(content)))
        self.end_headers()
        self.wfile.write(content)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, default=28680)
    args = parser.parse_args()
    ThreadingHTTPServer(("127.0.0.1", args.port), Handler).serve_forever()
