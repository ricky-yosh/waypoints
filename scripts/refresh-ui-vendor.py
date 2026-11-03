#!/usr/bin/env python3
"""Regenerate pinned UI dependency sources and apply our manifest-only patches."""

import hashlib
import io
from pathlib import Path
import re
import shutil
import tarfile
import urllib.request


ROOT = Path(__file__).resolve().parents[1]
CRATES = [
    ("gpui-kit", "0.9.0", "2c464934b5cdb85dd3fc511e60d09fb446fba422de159a80038368f52893c14b"),
    ("gpui-pre", "0.3.11", "9303c771abf2acff2a96d706f69d9a82088c4770173a07cce88f0002431d887d"),
    ("gpui-pre-macos", "0.3.11", "d7410cfdaa1fa55a414346cc492d18ce340e3a8b6d0ac3213b984ed690c82b92"),
    ("gpui-pre-linux", "0.3.11", "79cac5cbd3485c1e3db8eafd5dc69019188f5cb0a39846946af772efa386f624"),
    ("gpui-pre-windows", "0.3.11", "5141a3db59bab761a16ab19529fe4117a86e91922e8fc00af6dfc4829cb1744d"),
]


def keep_source(name, relative):
    if name == "gpui-pre" and relative.parts[0] == "examples":
        # platform.rs includes this image in a unit test.
        return relative == Path("examples/image/exif-orientation-rotate-180.jpg")
    return True


def patch_manifest(name, text):
    if name == "gpui-pre":
        text = re.sub(r"(?ms)^\[\[example\]\]\n.*?(?=^\[|\Z)", "", text)
    if name == "gpui-kit":
        return text.replace("[features]\n", '[features]\nhot-reload = ["gpui/hot-reload"]\n', 1)
    versions = {
        "accesskit": "0.25.2",
        "accesskit_macos": "0.27.2",
        "accesskit_unix": "0.24.1",
        "accesskit_windows": "0.35.2",
    }
    for dependency, version in versions.items():
        pattern = rf'(\[[^\]\n]*dependencies\.{dependency}\]\nversion = ")[^"]+(")'
        text = re.sub(pattern, lambda match: match[1] + version + match[2], text)
    return text


def main():
    # Download and verify everything before replacing generated source directories.
    archives = []
    for name, version, checksum in CRATES:
        url = f"https://crates.io/api/v1/crates/{name}/{version}/download"
        with urllib.request.urlopen(url, timeout=60) as response:
            data = response.read()
        if hashlib.sha256(data).hexdigest() != checksum:
            raise ValueError(f"checksum mismatch for {name} {version}")
        archives.append((name, version, data))

    for name, version, data in archives:
        destination = ROOT / "vendor" / name
        if destination.exists():
            shutil.rmtree(destination)
        destination.mkdir(parents=True)
        prefix = f"{name}-{version}/"
        with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as archive:
            for member in archive.getmembers():
                if not member.isfile():
                    continue
                if not member.name.startswith(prefix):
                    raise ValueError(f"unexpected archive path: {member.name}")
                relative = Path(member.name.removeprefix(prefix))
                if relative.is_absolute() or ".." in relative.parts:
                    raise ValueError(f"unsafe archive path: {member.name}")
                # Cargo's archive bookkeeping and upstream lockfiles aren't sources.
                if relative.name in {".cargo_vcs_info.json", ".cargo-ok", "Cargo.lock"}:
                    continue
                if not keep_source(name, relative):
                    continue
                path = destination / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(archive.extractfile(member).read())
        manifest = destination / "Cargo.toml"
        manifest.write_text(patch_manifest(name, manifest.read_text()))
        print(f"Generated vendor/{name} from {version}")


if __name__ == "__main__":
    main()
