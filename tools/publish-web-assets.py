#!/usr/bin/env python3
"""Publishes the files bundled through `asset_bytes!` for the browser build.

    python3 tools/publish-web-assets.py OUT_DIR [--generated DIR]

Every `vearth_platform::asset_bytes!` call site in the workspace is resolved
to a repository-relative path; unrecognized call forms fail the build, so a
new asset cannot be missed. Every file under the --generated directory is
published as `generated/<path relative to DIR>`. Each file is written under
OUT_DIR/assets/ at that path, gzip-compressed when that saves at least a
tenth, and split into consecutive `.NNN` parts when larger than the static
host's per-file limit. Files read through `asset_blocks!` are instead
published as fixed-size `.bNNNNN` blocks, each compressed on its own.
OUT_DIR/asset-manifest.json records the uncompressed size, a content hash
(used as the cache-busting query), compression, part count and block size.
Compressed payloads are cached by content hash under target/web-asset-cache.
"""

import gzip
import hashlib
import json
import pathlib
import re
import shutil
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
CACHE = ROOT / "target" / "web-asset-cache"
# Cloudflare Pages rejects files over 25 MiB.
PART_BYTES = 20 * 1024 * 1024

# Uncompressed bytes per `asset_blocks!` block.
BLOCK_BYTES = 64 * 1024

CALL = re.compile(r"asset_bytes!\(")
LITERAL = re.compile(r'asset_bytes!\(\s*"([^"]+)"\s*,?\s*\)')
BLOCKS = re.compile(r'asset_blocks!\(\s*"([^"]+)"\s*,?\s*\)')
# `asset_bytes!(concat!("dir/", $name, ".ext"))` inside a macro_rules table.
CONCAT = re.compile(r'asset_bytes!\(\s*concat!\(\s*"([^"]+)",\s*\$name,\s*"([^"]+)",?\s*\)\s*,?\s*\)')


def referenced_assets():
    """Paths read whole and paths read in blocks."""
    paths = set()
    blocks = set()
    for source in sorted((ROOT / "crates").glob("*/src/**/*.rs")):
        text = source.read_text()
        if "macro_rules! asset_bytes" in text:
            continue  # the macros' own definitions and documentation
        block_calls = text.count("asset_blocks!(")
        for match in BLOCKS.finditer(text):
            blocks.add((source.parent / match.group(1)).resolve())
            block_calls -= 1
        if block_calls:
            sys.exit(f"{source}: {block_calls} asset_blocks! calls not understood")
        calls = len(CALL.findall(text))
        if not calls:
            continue
        found = 0
        for match in LITERAL.finditer(text):
            paths.add((source.parent / match.group(1)).resolve())
            found += 1
        for match in CONCAT.finditer(text):
            directory = (source.parent / match.group(1)).resolve()
            suffix = match.group(2)
            matches = sorted(directory.glob("*" + suffix))
            if not matches:
                sys.exit(f"{source}: no {suffix} files in {directory}")
            paths.update(path.resolve() for path in matches)
            found += 1
        if found != calls:
            sys.exit(f"{source}: {calls} asset_bytes! calls, {found} understood")
    missing = [path for path in paths | blocks if not path.is_file()]
    if missing:
        sys.exit("missing assets:\n" + "\n".join(map(str, missing)))
    if paths & blocks:
        sys.exit("read both whole and in blocks:\n" + "\n".join(map(str, paths & blocks)))
    return sorted(paths), sorted(blocks)


def compressed(data, digest):
    CACHE.mkdir(parents=True, exist_ok=True)
    cached = CACHE / (digest + ".gz")
    if not cached.exists():
        # mtime=0 keeps the output reproducible.
        cached.write_bytes(gzip.compress(data, compresslevel=9, mtime=0))
    return cached.read_bytes()


def main():
    args = sys.argv[1:]
    generated = None
    if len(args) == 3 and args[1] == "--generated":
        generated = pathlib.Path(args[2]).resolve()
    elif len(args) != 1:
        sys.exit(__doc__)
    out = pathlib.Path(args[0]) / "assets"
    if out.exists():
        shutil.rmtree(out)
    whole, blocks = referenced_assets()
    sources = [(path.relative_to(ROOT).as_posix(), path) for path in whole]
    if generated is not None:
        sources += [
            ("generated/" + path.relative_to(generated).as_posix(), path)
            for path in sorted(generated.rglob("*"))
            if path.is_file()
        ]
    manifest = {}
    total_source = total_published = 0
    for relative, path in sources:
        data = path.read_bytes()
        digest = hashlib.sha256(data).hexdigest()
        payload = data
        packed = compressed(data, digest)
        use_gzip = len(packed) <= len(data) * 0.9
        if use_gzip:
            payload = packed
        name = relative + (".gz" if use_gzip else "")
        target = out / name
        target.parent.mkdir(parents=True, exist_ok=True)
        parts = 0
        if len(payload) > PART_BYTES:
            parts = (len(payload) + PART_BYTES - 1) // PART_BYTES
            for index in range(parts):
                chunk = payload[index * PART_BYTES : (index + 1) * PART_BYTES]
                target.with_name(f"{target.name}.{index:03}").write_bytes(chunk)
        else:
            target.write_bytes(payload)
        manifest[relative] = {
            "size": len(data),
            "version": digest[:16],
            "gzip": use_gzip,
            "parts": parts,
        }
        total_source += len(data)
        total_published += len(payload)
    for path in blocks:
        relative = path.relative_to(ROOT).as_posix()
        data = path.read_bytes()
        digest = hashlib.sha256(data).hexdigest()
        chunks = [data[i : i + BLOCK_BYTES] for i in range(0, len(data), BLOCK_BYTES)]
        packed = [compressed(chunk, hashlib.sha256(chunk).hexdigest()) for chunk in chunks]
        use_gzip = sum(map(len, packed)) <= len(data) * 0.9
        payloads = packed if use_gzip else chunks
        target = out / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        for index, payload in enumerate(payloads):
            name = f"{target.name}.b{index:05}" + (".gz" if use_gzip else "")
            target.with_name(name).write_bytes(payload)
        manifest[relative] = {
            "size": len(data),
            "version": digest[:16],
            "gzip": use_gzip,
            "block": BLOCK_BYTES,
        }
        total_source += len(data)
        total_published += sum(map(len, payloads))
    (out.parent / "asset-manifest.json").write_text(json.dumps(manifest, indent=1, sort_keys=True))
    print(
        f"published {len(manifest)} assets: {total_source / 2**20:.0f} MiB "
        f"as {total_published / 2**20:.0f} MiB"
    )


if __name__ == "__main__":
    main()
