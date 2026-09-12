#!/usr/bin/env python3
"""Create only synthetic image/archive files for the loopback SMB fixture."""

import binascii
import pathlib
import struct
import zipfile
import zlib

ROOT = pathlib.Path("/tmp/manga-reader-smb-fixture")

def png_chunk(kind, content):
    return struct.pack(">I", len(content)) + kind + content + struct.pack(">I", binascii.crc32(kind + content) & 0xFFFFFFFF)


def blank_png(width=320, height=240):
    rows = b"".join(b"\0" + b"\xff\xff\xff\xff" * width for _ in range(height))
    return b"\x89PNG\r\n\x1a\n" + png_chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)) + png_chunk(b"IDAT", zlib.compress(rows)) + png_chunk(b"IEND", b"")


PNG = blank_png()

for name in ("lock", "state", "cache", "private", "ncalrpc", "share/シリーズ弐/Vol 2"):
    (ROOT / name).mkdir(parents=True, exist_ok=True)
(ROOT / "share/シリーズ弐/Vol 2/001.webp").write_bytes(PNG)
(ROOT / "share/シリーズ弐/Vol 2/002.JPEG").write_bytes(PNG)
archive_path = ROOT / "share/シリーズ弐/Vol 10.CBZ"
with zipfile.ZipFile(archive_path, "w", zipfile.ZIP_DEFLATED) as archive:
    archive.writestr("010.png", PNG)
    archive.writestr("002.png", PNG)
