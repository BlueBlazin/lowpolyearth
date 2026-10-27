#!/usr/bin/env python3
"""Copy a Protomaps daily build into the site's bucket, verify or delete one.

Streams the build from build.protomaps.com into the R2 bucket as 1 MiB
block objects through R2's S3 API; reruns skip blocks already copied. Run it
where bandwidth is plentiful: the "Copy Protomaps build" GitHub workflow
does (see web/README.md).

    python3 tools/copy-protomaps-build.py [BUILD]         copy, then verify
    python3 tools/copy-protomaps-build.py --verify BUILD  compare samples only
    python3 tools/copy-protomaps-build.py --delete BUILD  remove a copied build

BUILD is a YYYYMMDD tag and defaults to the newest published build. Copying
and deleting need R2_ACCOUNT_ID (the Cloudflare account holding the bucket),
R2_ACCESS_KEY_ID and R2_SECRET_ACCESS_KEY (an R2 API token with object read
and write access to the bucket) and boto3.
"""

import argparse
import concurrent.futures
import hashlib
import json
import os
import random
import sys
import threading
import time
import urllib.error
import urllib.request

BUCKET = "lowpolyearth-maps"
BUILDS = "https://build-metadata.protomaps.dev/builds.json"
SOURCE = "https://build.protomaps.com"
HOSTED = "https://maps.lowpolyearth.online/protomaps"
# Must match HOSTED_BUILD_BLOCK_BYTES in crates/vearth-data/src/vectors.rs.
BLOCK = 1024 * 1024
# Blocks fetched from the source per range request.
SPAN = 4
# The source stalls some requests indefinitely; one this slow is retried.
SPAN_DEADLINE_S = 45
CACHE_CONTROL = "public, max-age=31536000, immutable"
USER_AGENT = "lowpolyearth-protomaps-copy"


class Stalled(Exception):
    pass


def read_within(response, deadline):
    chunks = []
    while chunk := response.read(256 * 1024):
        chunks.append(chunk)
        if time.monotonic() > deadline:
            raise Stalled()
    return b"".join(chunks)


RETRIES = {"count": 0, "last": ""}


def get(url, headers=None, attempts=20, deadline_s=None):
    request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT, **(headers or {})})
    for attempt in range(attempts):
        try:
            with urllib.request.urlopen(request, timeout=15) as response:
                if deadline_s is None:
                    return response.read()
                return read_within(response, time.monotonic() + deadline_s)
        except (urllib.error.URLError, TimeoutError, ConnectionError, Stalled) as error:
            # The source answers 5xx now and then under sustained load, and
            # stalls some requests.
            RETRIES["count"] += 1
            RETRIES["last"] = f"{type(error).__name__}: {error}"
            if attempt == attempts - 1:
                raise
            time.sleep(min(2**attempt, 10))


def published_builds():
    return {entry["key"].removesuffix(".pmtiles"): entry for entry in json.loads(get(BUILDS))}


def prefix(build):
    return f"protomaps/{build}/"


def bucket():
    import boto3
    from botocore.config import Config

    return boto3.client(
        "s3",
        endpoint_url=f"https://{os.environ['R2_ACCOUNT_ID']}.r2.cloudflarestorage.com",
        aws_access_key_id=os.environ["R2_ACCESS_KEY_ID"],
        aws_secret_access_key=os.environ["R2_SECRET_ACCESS_KEY"],
        region_name="auto",
        config=Config(
            max_pool_connections=128,
            retries={"max_attempts": 5, "mode": "standard"},
            connect_timeout=10,
            read_timeout=60,
        ),
    )


def stored(client, build):
    """Sizes of the build's objects already in the bucket, by block index."""
    sizes = {}
    for page in client.get_paginator("list_objects_v2").paginate(Bucket=BUCKET, Prefix=prefix(build)):
        for item in page.get("Contents", []):
            sizes[int(item["Key"].rsplit("/", 1)[1].removesuffix(".bin"))] = item["Size"]
    return sizes


def copy(build, size, parallel, shard, shards):
    client = bucket()
    # Fails in seconds, rather than after retries, when uploads cannot work.
    started = time.monotonic()
    status_key = f"protomaps/{build}.progress.{shard}"
    client.put_object(Bucket=BUCKET, Key=status_key, Body=b"starting")
    print(f"test upload took {time.monotonic() - started:.1f} s", flush=True)
    blocks = -(-size // BLOCK)
    length = lambda index: min(BLOCK, size - index * BLOCK)
    done = stored(client, build)
    missing = [index for index in range(blocks) if done.get(index) != length(index)]
    spans = sorted({index // SPAN for index in missing if index // SPAN % shards == shard})
    missing = set(missing)
    print(f"{len(missing)} of {blocks} blocks to copy in {len(spans)} requests", flush=True)
    started = time.monotonic()
    progress = {"spans": 0, "bytes": 0}
    finished = threading.Event()

    def report():
        elapsed = time.monotonic() - started
        line = (
            f"{progress['spans']}/{len(spans)} requests, {progress['bytes'] / 1e9:.1f} GB, "
            f"{progress['bytes'] / 1e6 / max(elapsed, 1):.0f} MB/s, {elapsed / 60:.1f} min, "
            f"{RETRIES['count']} retries (last: {RETRIES['last'][:160]})"
        )
        print(line, flush=True)
        # Readable at HOSTED/<build>.progress.<shard> while the copy runs.
        client.put_object(
            Bucket=BUCKET, Key=status_key, Body=line.encode(),
            CacheControl="no-store", ContentType="text/plain",
        )

    def reporter():
        while not finished.wait(30):
            report()

    threading.Thread(target=reporter, daemon=True).start()

    def run(span):
        first = span * SPAN
        last = min(first + SPAN, blocks) - 1
        start, end = first * BLOCK, last * BLOCK + length(last) - 1
        for attempt in range(10):
            data = get(
                f"{SOURCE}/{build}.pmtiles", {"Range": f"bytes={start}-{end}"},
                deadline_s=SPAN_DEADLINE_S,
            )
            if len(data) == end - start + 1:
                break
            # The source sometimes closes a response early.
            RETRIES["count"] += 1
            RETRIES["last"] = f"short body: {len(data)} of {end - start + 1} bytes"
        else:
            raise RuntimeError(f"blocks {first}-{last}: got {len(data)} bytes")
        for index in range(first, last + 1):
            if index in missing:
                offset = (index - first) * BLOCK
                client.put_object(
                    Bucket=BUCKET,
                    Key=f"{prefix(build)}{index}.bin",
                    Body=data[offset : offset + length(index)],
                    ContentType="application/octet-stream",
                    CacheControl=CACHE_CONTROL,
                )
        progress["spans"] += 1
        progress["bytes"] += len(data)

    report()
    pool = concurrent.futures.ThreadPoolExecutor(parallel)
    try:
        for future in concurrent.futures.as_completed([pool.submit(run, span) for span in spans]):
            future.result()
    finally:
        # Stop at the first failure instead of working through the queue.
        pool.shutdown(cancel_futures=True)
    finished.set()
    report()


def verify(build, size, samples):
    blocks = -(-size // BLOCK)
    chosen = sorted({0, blocks - 1, *random.sample(range(blocks), min(samples, blocks))})
    for index in chosen:
        start = index * BLOCK
        length = min(BLOCK, size - start)
        hosted = get(f"{HOSTED}/{build}/{index}.bin")
        source = get(f"{SOURCE}/{build}.pmtiles", {"Range": f"bytes={start}-{start + length - 1}"})
        if len(hosted) != length or hashlib.sha256(hosted).digest() != hashlib.sha256(source).digest():
            sys.exit(f"block {index} differs from the source")
    print(f"verified {len(chosen)} of {blocks} blocks of {build} against the source")


def delete(build):
    client = bucket()
    keys = [f"{prefix(build)}{index}.bin" for index in stored(client, build)]
    for start in range(0, len(keys), 1000):
        client.delete_objects(
            Bucket=BUCKET,
            Delete={"Objects": [{"Key": key} for key in keys[start : start + 1000]], "Quiet": True},
        )
    print(f"deleted {len(keys)} objects of {build}")


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("build", nargs="?", help="YYYYMMDD; default: newest published")
    parser.add_argument("--parallel", type=int, default=48, help="concurrent source requests")
    parser.add_argument(
        "--shard", default="0/1", help="K/N: copy every Nth source request from the Kth; "
        "the source limits each client's throughput, so shards run on separate machines"
    )
    parser.add_argument("--samples", type=int, default=24, help="blocks compared with the source")
    parser.add_argument("--verify", action="store_true", help="only compare samples with the source")
    parser.add_argument("--delete", action="store_true", help="delete the hosted build")
    args = parser.parse_args()

    if args.delete:
        if not args.build:
            sys.exit("--delete needs a build")
        delete(args.build)
        return
    builds = published_builds()
    build = args.build or max(builds)
    if build not in builds:
        sys.exit(f"build {build} is not published; published: {', '.join(sorted(builds)[-7:])}")
    size = builds[build]["size"]
    if not args.verify:
        print(f"copying {build}: {size / 1e9:.1f} GB", flush=True)
        shard, shards = (int(part) for part in args.shard.split("/"))
        copy(build, size, args.parallel, shard, shards)
    if args.shard == "0/1" or args.verify:
        verify(build, size, args.samples)
    print(f"to use it, set PROTOMAPS_BUILD = \"{build}\" in crates/vearth-data/src/vectors.rs")


if __name__ == "__main__":
    main()
