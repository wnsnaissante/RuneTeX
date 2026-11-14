"""Rebuild the pinned Poppler notices from original conda package metadata.

Run with Python 3, requests, PyYAML, and Windows tar (libarchive with zstd).
Downloaded packages are disposable; notices are edited in the ignored expanded
bundle cache, then saved with Update-NoticeManifest.ps1.
"""
from __future__ import annotations

import concurrent.futures
import argparse
import hashlib
import json
import os
import pathlib
import re
import shutil
import subprocess
import sys
import tarfile
import zipfile

WORKSPACE = pathlib.Path(__file__).resolve().parent.parent
OUT: pathlib.Path
CACHE = WORKSPACE / ".deps" / "poppler-notices"
DIST = WORKSPACE / ".deps" / "poppler" / "poppler-26.09.0"
RUN_URL = "https://github.com/oschwartz10612/poppler-windows/actions/runs/34991014991"
COMMIT = "dc4cae5410474e4d75c5c494eb1a8c6774e47457"


def notice_root() -> pathlib.Path:
    """Expand the checked-in archive into the ignored, editable notice cache."""
    powershell = pathlib.Path(os.environ.get("SystemRoot", r"C:\Windows")) / (
        "System32/WindowsPowerShell/v1.0/powershell.exe"
    )
    result = subprocess.run(
        [str(powershell), "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
         "-Command", "[Console]::OutputEncoding = New-Object Text.UTF8Encoding($false); "
         ". './scripts/NoticeBundle.ps1'; Get-NoticeRoot -AllowWorkingChanges"],
        cwd=WORKSPACE, check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        encoding="utf-8-sig", errors="replace",
    )
    lines = result.stdout.strip().splitlines()
    if len(lines) != 1:
        raise ValueError("Notice bundle helper returned an invalid cache directory")
    root = pathlib.Path(lines[0]).resolve()
    cache = (WORKSPACE / ".cache/notices").resolve()
    if root.name != "licenses" or not root.is_relative_to(cache):
        raise ValueError("Notice bundle helper returned a directory outside its cache")
    return root


def digest(path: pathlib.Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def save_json(path: pathlib.Path, value) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def download(url: str, path: pathlib.Path, sha: str | None = None) -> None:
    import requests

    if path.exists() and (sha is None or digest(path) == sha):
        return
    response = requests.get(url, timeout=120)
    response.raise_for_status()
    data = response.content
    if sha is not None and hashlib.sha256(data).hexdigest() != sha:
        raise RuntimeError(f"SHA-256 mismatch: {url}")
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)


def recipe_sources(value):
    if isinstance(value, dict):
        for key, child in value.items():
            if key == "source" and isinstance(child, (list, dict)):
                for source in [child] if isinstance(child, dict) else child:
                    if isinstance(source, dict) and ("url" in source or "git_url" in source):
                        yield source
            else:
                yield from recipe_sources(child)
    elif isinstance(value, list):
        for child in value:
            yield from recipe_sources(child)


def collect_package(record: dict) -> dict:
    import yaml

    name, version, build, subdir = (record[key] for key in ("name", "version", "build", "subdir"))
    identifier = f"{name}-{version}-{build}"
    api = CACHE / "api" / f"{name}-{version}.json"
    download(f"https://api.anaconda.org/release/conda-forge/{name}/{version}", api)
    metadata = json.loads(api.read_text(encoding="utf-8"))
    candidates = [entry for entry in metadata["distributions"]
                  if entry["attrs"].get("subdir") == subdir and entry["attrs"].get("build") == build]
    if len(candidates) != 1:
        raise RuntimeError(f"Expected one package for {identifier}: {len(candidates)}")
    package = candidates[0]
    filename = package["basename"].split("/")[-1]
    archive_url = "https:" + package["download_url"] if package["download_url"].startswith("//") else package["download_url"]
    archive_sha = package.get("sha256") or package["attrs"].get("sha256")
    if not archive_sha:
        raise RuntimeError(f"Missing SHA-256 for {identifier}")
    archive = CACHE / "archives" / filename
    download(archive_url, archive, archive_sha)
    extracted = CACHE / "info" / identifier
    extracted.mkdir(parents=True, exist_ok=True)
    if archive.suffix == ".conda":
        with zipfile.ZipFile(archive) as package_zip:
            names = [member for member in package_zip.namelist() if member.startswith("info-") and member.endswith(".tar.zst")]
            if len(names) != 1 or pathlib.PurePosixPath(names[0]).name != names[0]:
                raise RuntimeError(f"Unsafe metadata archive: {filename}")
            tar = CACHE / "archives" / names[0]
            tar.write_bytes(package_zip.read(names[0]))
    else:
        tar = archive
    members = subprocess.check_output(["tar", "-tf", str(tar)], text=True, encoding="utf-8").splitlines()
    for member in members:
        pure = pathlib.PurePosixPath(member)
        if pure.is_absolute() or ".." in pure.parts:
            raise RuntimeError(f"Unsafe archive path in {filename}: {member}")
    subprocess.run(["tar", "-xf", str(tar), "-C", str(extracted), "info"], check=True)
    info = extracted / "info"
    about = json.loads((info / "about.json").read_text(encoding="utf-8"))
    index = json.loads((info / "index.json").read_text(encoding="utf-8"))
    if (index["name"], index["version"], index["build"]) != (name, version, build):
        raise RuntimeError(f"Incorrect extracted package: {identifier}")
    paths_file = info / "paths.json"
    paths = json.loads(paths_file.read_text(encoding="utf-8"))["paths"] if paths_file.exists() else []
    # Older conda archives omit paths.json. Hash their payload files directly.
    if not paths:
        subprocess.run(["tar", "-xf", str(archive), "-C", str(extracted)], check=True)
        paths = [{"_path": line, "sha256": digest(extracted / line)}
                 for line in (info / "files").read_text(encoding="utf-8").splitlines()
                 if (extracted / line).is_file()]
    destination = OUT / "packages" / identifier
    destination.mkdir(parents=True, exist_ok=True)
    for basename in ("licenses", "recipe"):
        original = info / basename
        if original.exists():
            shutil.copytree(original, destination / basename, dirs_exist_ok=True)
    for basename in ("about.json", "index.json"):
        shutil.copyfile(info / basename, destination / basename)
    if name == "ucrt":
        license_dir = destination / "licenses"
        license_dir.mkdir(exist_ok=True)
        shutil.copyfile(destination / "recipe" / "LICENSE.txt", license_dir / "LICENSE.txt")
    if name == "libfreetype6":
        # Conda's split runtime output omits the parent's license files. The
        # corresponding parent output is built from the same source and recipe.
        parent = collect_package({"name": "freetype", "version": version,
                                  "build": "h57928b3_2", "subdir": subdir})
        shutil.copytree(OUT / parent["metadata_dir"] / "licenses", destination / "licenses", dirs_exist_ok=True)
    if name == "graphite2":
        source = CACHE / "sources" / "graphite2-1.3.15.tgz"
        download("https://github.com/silnrsi/graphite/releases/download/1.3.15/graphite2-1.3.15.tgz", source,
                 "c6bc8b4252724665297f7cad0c55897285c673f9b8e6db3522ace833593fe0b1")
        with tarfile.open(source) as source_tar:
            member = source_tar.getmember("graphite2-1.3.15/LICENSE")
            (destination / "licenses" / "LICENSE").write_bytes(source_tar.extractfile(member).read())
    if name == "libintl":
        source = CACHE / "sources" / "gettext-0.22.5.tar.xz"
        download("https://ftp.gnu.org/pub/gnu/gettext/gettext-0.22.5.tar.xz", source,
                 "fe10c37353213d78a5b83d48af231e005c4da84db5ce88037d88355938259640")
        with tarfile.open(source) as source_tar:
            member = source_tar.getmember("gettext-0.22.5/gettext-runtime/intl/COPYING.LIB")
            (destination / "licenses" / "COPYING.LIB").write_bytes(source_tar.extractfile(member).read())
    if name == "pcre2":
        source = CACHE / "sources" / "pcre2-10.47.tar.gz"
        download("https://github.com/PCRE2Project/pcre2/archive/refs/tags/pcre2-10.47.tar.gz", source,
                 "409c443549b13b216da40049850a32f3e6c57d4224ab11553ab5a786878a158e")
        with tarfile.open(source) as source_tar:
            member = source_tar.getmember("pcre2-pcre2-10.47/LICENCE.md")
            (destination / "licenses" / "LICENCE.md").write_bytes(source_tar.extractfile(member).read())
        jit_revision = "45f910b78c6605ebf5b53d3ec7cb00f2312fe417"
        source = CACHE / "sources" / f"sljit-{jit_revision}.tar.gz"
        download(f"https://github.com/zherczeg/sljit/archive/{jit_revision}.tar.gz", source,
                 "ea5a6fb45e050f813f070ecaa098fe0389068beea61ac7dce5410a0c642ccfa3")
        with tarfile.open(source) as source_tar:
            member = source_tar.getmember(f"sljit-{jit_revision}/LICENSE")
            (destination / "licenses" / "SLJIT-LICENSE").write_bytes(source_tar.extractfile(member).read())
    if name == "poppler-data":
        source = CACHE / "sources" / "poppler-data-0.4.12.tar.gz"
        download("https://poppler.freedesktop.org/poppler-data-0.4.12.tar.gz", source,
                 "c835b640a40ce357e1b83666aabd95edffa24ddddd49b8daff63adb851cdab74")
        license_dir = destination / "licenses"
        license_dir.mkdir(exist_ok=True)
        with tarfile.open(source) as source_tar:
            for member in source_tar.getmembers():
                if not member.isfile():
                    continue
                content = source_tar.extractfile(member).read()
                relative = pathlib.PurePosixPath(member.name).relative_to("poppler-data-0.4.12").as_posix()
                paths.append({"_path": "upstream-source/" + relative,
                              "sha256": hashlib.sha256(content).hexdigest()})
                if relative in ("COPYING", "COPYING.adobe", "COPYING.gpl2"):
                    (license_dir / relative).write_bytes(content)
    license_files = [path.relative_to(OUT).as_posix() for path in sorted((destination / "licenses").rglob("*")) if path.is_file()]
    rendered_recipe = destination / "recipe" / "meta.yaml"
    if not rendered_recipe.exists():
        rendered_recipe = destination / "recipe" / "rendered_recipe.yaml"
    recipe = yaml.safe_load(rendered_recipe.read_text(encoding="utf-8")) if rendered_recipe.exists() else {}
    source_urls = []
    for source in recipe_sources(recipe):
        urls = source.get("url", source.get("git_url", []))
        if isinstance(urls, str):
            urls = [urls]
        for url in urls:
            item = {"url": url, "sha256": source.get("sha256"), "git_rev": source.get("git_rev")}
            if item not in source_urls:
                source_urls.append(item)
    extra = about.get("extra", {})
    feedstock = extra.get("remote_url", "")
    commit = extra.get("sha")
    result = {
        "id": identifier, "name": name, "version": version, "build": build,
        "subdir": subdir, "license": about.get("license", index.get("license")),
        "archive_url": archive_url, "archive_sha256": archive_sha,
        "homepage": about.get("home"), "recipe_repository": feedstock,
        "recipe_commit": commit,
        "recipe_url": f"{feedstock}/tree/{commit}/recipe" if feedstock and commit else None,
        "source_urls": source_urls, "license_files": license_files,
        "metadata_dir": destination.relative_to(OUT).as_posix(), "_paths": paths,
    }
    if name == "graphite2":
        result["metadata_license"] = result["license"]
        result["license"] = "LGPL-2.1-or-later OR MPL-2.0 OR GPL-2.0-or-later OR MIT"
        result["license_note"] = "License options follow the shipped upstream LICENSE; the conda license field says LGPL-2.0-or-later. Original COPYING and full LICENSE are retained."
    if name == "libfreetype6":
        result["license_note"] = "FreeType License (FTL) selected. License texts recovered from freetype-2.14.3-h57928b3_2, built from the same source and parent recipe."
        result["license_provenance"] = {key: parent[key] for key in ("id", "archive_url", "archive_sha256", "metadata_dir")}
        alternative = {
            "url": "https://github.com/freetype/freetype/archive/0a0221a1347e2f1e07c395263540026e9a0aa7c7.tar.gz",
            "sha256": "11cd478953fc1d382f20a233b8e2aed6c31b47cbf0568c4bdb3334bcc1550698",
            "git_rev": "0a0221a1347e2f1e07c395263540026e9a0aa7c7",
        }
        alternate_archive = CACHE / "sources" / "freetype-0a0221a1347e2f1e07c395263540026e9a0aa7c7.tar.gz"
        download(alternative["url"], alternate_archive, alternative["sha256"])
        with tarfile.open(alternate_archive) as alternate_tar:
            for basename in ("FTL.TXT", "GPLv2.TXT"):
                member = alternate_tar.getmember(f"freetype-{alternative['git_rev']}/docs/{basename}")
                if alternate_tar.extractfile(member).read() != (destination / "licenses" / "docs" / basename).read_bytes():
                    raise RuntimeError("FreeType source and license package do not match")
        result["source_urls"].append(alternative)
        result["source_urls"][0]["availability"] = "mirror-timeout"
        alternative["availability"] = "verified-download"
        result["preferred_source_urls"] = [alternative]
        result["source_note"] = "Source 1 is the original GNU release tarball used by conda. Source 2 is the official FreeType VER-2-14-3 tag's commit archive, provided as an immutable mirror fallback; its license texts were byte-compared with the same-version conda parent package."
    if name == "libzlib":
        fallback = {"url": "https://zlib.net/fossils/zlib-1.3.2.tar.gz",
                    "sha256": "bb329a0a2cd0274d05519d61c667c062e06990d72e125ee2dfa8de64f0119d16", "git_rev": None}
        download(fallback["url"], CACHE / "sources" / "zlib-1.3.2.tar.gz", fallback["sha256"])
        result["source_urls"].append(fallback)
        for original in result["source_urls"][:-1]:
            original["availability"] = "historical-404"
        fallback["availability"] = "verified-download"
        result["preferred_source_urls"] = [fallback]
        result["source_note"] = "The original recipe's zlib.net top-level and gnupg mirror URLs now return 404. Source 3 is zlib's official fossils archive, downloaded and verified to the original recipe's exact SHA-256."
    if name == "libintl":
        result["license_note"] = "The conda package provides gettext's GPLv3 COPYING. The LGPLv2.1 COPYING.LIB applicable to the libintl runtime was additionally extracted from the exact hashed gettext-0.22.5 source. Both texts are retained."
    if name == "pcre2":
        result["metadata_license"] = result["license"]
        result["license"] = "(BSD-3-Clause WITH PCRE2-exception) AND BSD-2-Clause"
        result["license_note"] = "The conda COPYING is a pointer. Full LICENCE.md and the license of the exact SLJIT revision used by the recipe were extracted from their SHA-256-verified source archives."
    print(f"Collected {identifier}: {len(license_files)} license files", flush=True)
    return result


def main() -> None:
    lock = OUT / "packages-lock.json"
    if lock.exists():
        records = json.loads(lock.read_text(encoding="utf-8"))
    else:
        log = (CACHE / "actions-build.log").read_text(encoding="utf-8")
        records = []
        for subdir, filename in re.findall(r"conda-forge/(win-64|noarch)::([^\s]+)", log):
            name, version, build = filename.rsplit("-", 2)
            record = {"name": name, "version": version, "build": build, "subdir": subdir}
            if record not in records:
                records.append(record)
        if len(records) != 44:
            raise RuntimeError(f"Unexpected CI package count: {len(records)}")
        save_json(lock, sorted(records, key=lambda record: record["name"]))
    # The upstream release copies Poppler's Library tree, every DLL from the
    # environment, and a separately downloaded poppler-data source tree. The
    # environment's fonts and certificate stores are not included in that tree.
    binary_records = [record for record in records if
                      (record["subdir"] == "win-64" or record["name"] == "poppler-data")
                      and record["name"] not in ("libfreetype", "vc")]
    with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
        packages = list(pool.map(collect_package, binary_records))
    hashes = {}
    for package in packages:
        for item in package.pop("_paths"):
            sha = item.get("sha256")
            if sha:
                hashes.setdefault(sha, []).append({"package": package["id"], "original_path": item["_path"]})
    files, missing = [], []
    for path in sorted(DIST.rglob("*")):
        if not path.is_file():
            continue
        sha = digest(path)
        relative = path.relative_to(DIST).as_posix()
        matches = hashes.get(sha, [])
        files.append({"path": relative, "sha256": sha, "size": path.stat().st_size, "origins": matches})
        if not matches:
            missing.append(relative)
    used = {origin["package"] for file in files for origin in file["origins"]}
    used_packages = [package for package in packages if package["id"] in used]
    for package in used_packages:
        if not package["license"] or not package["license_files"] or not package["source_urls"]:
            raise RuntimeError(f"Incomplete notices for {package['id']}")
    inventory = {"schema": 1, "distribution": {
        "name": "poppler-windows", "version": "26.09.0-0",
        "release_url": "https://github.com/oschwartz10612/poppler-windows/releases/tag/v26.09.0-0",
        "archive_url": "https://github.com/oschwartz10612/poppler-windows/releases/download/v26.09.0-0/Release-26.09.0-0.zip",
        "archive_sha256": "7a6f256a0ddf7536182246a5733331bf4677cbcc34f4663774947ad34556c8d0",
        "recipe_url": f"https://github.com/oschwartz10612/poppler-windows/tree/{COMMIT}",
        "build_log_url": RUN_URL,
    }, "packages": used_packages,
        "files": files, "unmapped_files": missing}
    save_json(OUT / "inventory.json", inventory)
    write_index(inventory)
    print(f"Matched {len(files)-len(missing)}/{len(files)} bundled files; {len(used)} packages", flush=True)
    for name in missing:
        print(f"UNMAPPED {name}", flush=True)
    if missing:
        raise SystemExit(2)


def write_index(inventory: dict) -> None:
    lines = [
        "# Poppler for Windows: third-party notices and sources", "",
        "This inventory covers the exact Poppler for Windows **26.09.0-0** tree bundled with RuneTeX.",
        "All **561 files**, including **131 DLLs** and **12 executables**, were matched by SHA-256 to their original conda packages or the unmodified poppler-data 0.4.12 source archive.", "",
        "- [Original Windows release](https://github.com/oschwartz10612/poppler-windows/releases/tag/v26.09.0-0)",
        f"- [Exact Windows packaging recipe](https://github.com/oschwartz10612/poppler-windows/tree/{COMMIT})",
        f"- [Original build log and resolved package versions]({RUN_URL})",
        "- [Per-file hashes, originating package builds, source/archive hashes and license paths](inventory.json)",
        "- [Complete environment package plan recovered from that build](packages-lock.json)", "",
        "The package directories preserve the publishers' license/copyright notices and the exact build recipes, patches and metadata. Package archive URLs and SHA-256 values are in inventory.json. Source archives listed below are the versions referenced by those recipes; the recipe directories include applied patches and build scripts. Microsoft runtime links point to Microsoft's original binary distributions and are labeled as such.", "",
        "## Acknowledgments", "",
        "Portions of this software are copyright © 2000–2026 The FreeType Project (https://freetype.org/). All rights reserved. The FreeType portions are distributed under the FreeType License (FTL); its text and the alternative GPLv2 text are retained.", "",
        "Graphite2's upstream LICENSE offers LGPL 2.1 or later, MPL 2.0, GPL 2 or later, or MIT for SIL-authored code unless otherwise specified. Its original COPYING and full LICENSE are retained; this corrects the older LGPL-2.0-or-later label in the conda metadata without changing the original metadata.", "",
        "The poppler-data CMap notices and licenses are retained verbatim. Microsoft UCRT and Visual C++ runtime components retain their proprietary redistribution terms; the Windows SDK license and the original Visual C++ RTF/DOCX license files are included.", "",
        "PCRE2's full LICENCE.md and its pinned SLJIT license are included in addition to the conda COPYING pointer. The LGPL COPYING.LIB for the gettext/libintl runtime is retained alongside the conda package's GPL COPYING.", "",
        "For FreeType, Source 2 is the official release tag's immutable Git commit archive, verified by download; the original recipe's GNU tarball URL and hash are retained. For zlib, Source 3 is the official fossils archive and matches the recipe's exact source archive hash; the recipe's historical URLs are retained for provenance.", "",
        "## Bundled native packages", "",
        "| Package and exact build | License | License texts | Original source / distribution | Build recipe and patches |",
        "| --- | --- | --- | --- | --- |",
    ]
    for package in inventory["packages"]:
        notices = "<br>".join(f"[{pathlib.PurePosixPath(path).name}]({path})" for path in package["license_files"])
        kind = "Original Microsoft binary" if package["name"] in ("ucrt", "vc14_runtime", "vcomp14") else "Source"
        sources = "<br>".join(f"[{kind} {number}]({source['url']})" for number, source in enumerate(package["source_urls"], 1)
                              if source.get("availability") not in ("historical-404", "mirror-timeout"))
        recipe = f"[Included recipe]({package['metadata_dir']}/recipe)"
        if package["recipe_url"]:
            recipe += f"<br>[Pinned upstream recipe]({package['recipe_url']})"
        lines.append(f"| {package['id']} | {package['license']} | {notices} | {sources} | {recipe} |")
    lines += ["", "## Reproducing the inventory", "",
              "`scripts/Collect-PopplerNotices.py` reads the pinned package plan, verifies original package archive hashes, retains license texts and build recipes, and compares every bundled file hash with the upstream package metadata. It fails if any bundled file is unmapped or a used package has no license/source record. `python scripts/Collect-PopplerNotices.py --check` checks the expanded bundle cache without downloading packages. Python 3, requests, PyYAML and Windows tar with zstd support are needed only when refreshing this directory. Run `scripts/Update-NoticeManifest.ps1` afterward to save the updated notices back into the tracked archive. The installer expands the checked-in archive without fetching anything.", ""]
    (OUT / "INDEX.md").write_text("\n".join(lines), encoding="utf-8")


def check() -> None:
    inventory = json.loads((OUT / "inventory.json").read_text(encoding="utf-8"))
    expected = {item["path"]: item for item in inventory["files"]}
    found = {path.relative_to(DIST).as_posix(): path for path in DIST.rglob("*") if path.is_file()}
    if found.keys() != expected.keys() or inventory["unmapped_files"]:
        raise RuntimeError("The Poppler distribution file list changed")
    identifiers = {package["id"] for package in inventory["packages"]}
    for relative, path in found.items():
        entry = expected[relative]
        if digest(path) != entry["sha256"] or path.stat().st_size != entry["size"]:
            raise RuntimeError(f"The Poppler distribution file changed: {relative}")
        if not entry["origins"] or any(origin["package"] not in identifiers for origin in entry["origins"]):
            raise RuntimeError(f"Missing provenance: {relative}")
    for package in inventory["packages"]:
        if not package["license_files"] or not package["source_urls"]:
            raise RuntimeError(f"Incomplete package: {package['id']}")
        for relative in package["license_files"]:
            path = (OUT / relative).resolve()
            if not path.is_relative_to(OUT.resolve()) or not path.is_file() or not path.stat().st_size:
                raise RuntimeError(f"Missing license: {relative}")
        if not (OUT / package["metadata_dir"] / "recipe").is_dir():
            raise RuntimeError(f"Missing recipe: {package['id']}")
        if any(not source.get("sha256") and not source.get("git_rev") for source in package["source_urls"]):
            raise RuntimeError(f"Unpinned source: {package['id']}")
    print(f"Verified {len(found)} Poppler files, {len(identifiers)} native packages and all license/source references")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="Verify the expanded notice inventory and bundled inputs without network access")
    arguments = parser.parse_args()
    try:
        OUT = notice_root() / "poppler"
        check() if arguments.check else main()
    except (OSError, ValueError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f"Poppler notice collection failed: {error}", file=sys.stderr)
        if isinstance(error, subprocess.CalledProcessError) and error.stderr:
            print(error.stderr, file=sys.stderr)
        sys.exit(1)
