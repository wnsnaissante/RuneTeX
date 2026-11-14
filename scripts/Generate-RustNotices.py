#!/usr/bin/env python3
"""Collect pinned Windows Cargo notices from the local source cache (Python 3.11+).

No network requests or dependency changes are made. Upstream license files omitted
from a published crate are retained in the archived supplemental directory with
immutable commit URLs and SHA-256. Both application binaries share this Cargo
package's dependency graph. Build/proc-macro notices are conservatively retained.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
import tomllib
from pathlib import Path
from urllib.parse import quote


REPOSITORY = Path(__file__).resolve().parent.parent
TARGET = "x86_64-pc-windows-msvc"
NOTICE_NAME = re.compile(
    r"(?i)(?:^|[._-])(?:licen[cs]e|copying|copyright|notice|authors|ofl)(?:[._-]|$)"
)
SOURCE_SUFFIXES = {".rs", ".c", ".h", ".cpp", ".js", ".ts", ".py", ".toml"}
# Reviewed declarations in the two pinned Windows graphs. A dependency introducing
# another license requires explicit review instead of silently generating a claim.
REVIEWED_LICENSE_IDS = {
    "0BSD", "Apache-2.0", "BSD-2-Clause", "BSD-3-Clause", "BSL-1.0",
    "CC0-1.0", "ISC", "LLVM-exception", "MIT", "MIT-0", "MPL-2.0",
    "Unicode-3.0", "Unicode-DFS-2016", "Unlicense", "Zlib",
}


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def run(arguments: list[str]) -> str:
    result = subprocess.run(
        arguments, cwd=REPOSITORY, check=True, stdout=subprocess.PIPE,
        stderr=subprocess.PIPE, encoding="utf-8",
    )
    return result.stdout


def json_bytes(value: object) -> bytes:
    return (json.dumps(value, ensure_ascii=False, indent=2) + "\n").encode("utf-8")


def notice_root() -> Path:
    """Expand the checked-in archive into the ignored, editable notice cache."""
    powershell = Path(os.environ.get("SystemRoot", r"C:\Windows")) / (
        "System32/WindowsPowerShell/v1.0/powershell.exe"
    )
    result = subprocess.run(
        [str(powershell), "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
         "-Command", "[Console]::OutputEncoding = New-Object Text.UTF8Encoding($false); "
         ". './scripts/NoticeBundle.ps1'; Get-NoticeRoot -AllowWorkingChanges"],
        cwd=REPOSITORY, check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        encoding="utf-8-sig", errors="replace",
    )
    lines = result.stdout.strip().splitlines()
    if len(lines) != 1:
        raise ValueError("Notice bundle helper returned an invalid cache directory")
    root = Path(lines[0]).resolve()
    cache = (REPOSITORY / ".cache/notices").resolve()
    if root.name != "licenses" or not root.is_relative_to(cache):
        raise ValueError("Notice bundle helper returned a directory outside its cache")
    return root


def notice_candidates(directory: Path) -> list[Path]:
    return sorted(
        (
            path for path in directory.rglob("*")
            if path.is_file()
            and not path.is_symlink()
            and any(NOTICE_NAME.search(part) for part in path.relative_to(directory).parts)
            and path.suffix.lower() not in SOURCE_SUFFIXES
            and ".git" not in path.relative_to(directory).parts
        ),
        key=lambda path: path.relative_to(directory).as_posix().lower(),
    )


def git_root(directory: Path) -> Path:
    for parent in [directory, *directory.parents]:
        if (parent / ".git").exists():
            return parent
    raise ValueError(f"Cannot locate Git checkout for {directory.name}")


def git_source(source: str) -> tuple[str, str]:
    repository, revision = source.removeprefix("git+").rsplit("#", 1)
    repository = repository.split("?", 1)[0].removesuffix(".git")
    if not re.fullmatch(r"[0-9a-f]{40}", revision):
        raise ValueError(f"Unpinned Git source: {source}")
    return repository, revision


def safe_output(root: Path, relative: str) -> Path:
    path = root / relative
    if path.is_symlink() or not path.resolve().is_relative_to(root.resolve()):
        raise ValueError(f"Output path escapes notice directory: {relative}")
    return path


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest-path", type=Path, default=REPOSITORY / "Cargo.toml")
    parser.add_argument("--output", type=Path, help="Output under .cache/ (default: expanded notice bundle's rust directory)")
    parser.add_argument("--package", help="Root package in a virtual Cargo workspace")
    parser.add_argument("--skip-toolchain", action="store_true")
    parser.add_argument("--check", action="store_true", help="Verify generated files without writing")
    args = parser.parse_args()
    manifest = args.manifest_path.resolve()
    expanded = notice_root()
    supplemental_root = expanded / "rust/supplemental"
    output = args.output.resolve() if args.output else expanded / "rust"
    cache = (REPOSITORY / ".cache").resolve()
    if not output.is_relative_to(cache) or output == cache:
        raise ValueError("Output must be a dedicated directory inside the ignored .cache directory")
    common = ["--locked", "--offline", "--manifest-path", str(manifest)]
    metadata = json.loads(run([
        "cargo", "metadata", *common, "--format-version", "1", "--filter-platform", TARGET,
    ]))
    tree_command = [
        "cargo", "tree", *common, "--target", TARGET, "--edges", "normal,build",
        "--prefix", "none", "--format", "{p}",
    ]
    if args.package:
        tree_command += ["--package", args.package]
    tree = run(tree_command)
    selected = {
        (match.group(1), match.group(2))
        for line in tree.splitlines()
        if (match := re.match(r"(\S+) v(\S+)", line))
    }
    packages = [
        package for package in metadata["packages"]
        if (package["name"], package["version"]) in selected and package["source"]
    ]
    packages.sort(key=lambda package: (package["name"], package["version"], package["source"]))
    if not packages:
        raise ValueError("No source-backed Windows dependencies were found")
    keys = [(package["name"], package["version"]) for package in packages]
    if len(set(keys)) != len(keys):
        raise ValueError("Ambiguous name/version from multiple Cargo sources")
    workspace = Path(metadata["workspace_root"])
    lock = workspace / "Cargo.lock"
    locked_packages = tomllib.loads(lock.read_text(encoding="utf-8"))["package"]
    checksums = {
        (item["name"], item["version"], item.get("source")): item.get("checksum")
        for item in locked_packages
    }

    supplemental_manifest = json.loads((supplemental_root / "SOURCES.json").read_text(encoding="utf-8"))
    supplemental = supplemental_manifest["files"]
    for entry in supplemental:
        path = (supplemental_root.parent / entry["path"]).resolve()
        if not path.is_relative_to(supplemental_root.resolve()):
            raise ValueError("Supplemental path escapes its directory")
        if digest(path.read_bytes()) != entry["sha256"]:
            raise ValueError(f"Supplemental license was modified: {entry['path']}")

    generated: dict[str, bytes] = {}
    inventory_packages = []
    for package in packages:
        name, version = package["name"], package["version"]
        expression = package["license"]
        if not expression:
            raise ValueError(f"Missing or custom license requires review: {name} {version}")
        license_ids = set(re.findall(r"[A-Za-z0-9][A-Za-z0-9.+-]*", expression))
        unknown = license_ids - REVIEWED_LICENSE_IDS - {"AND", "OR", "WITH"}
        if unknown:
            raise ValueError(
                f"Unreviewed license for {name} {version}: " + ", ".join(sorted(unknown))
            )
        directory = Path(package["manifest_path"]).parent
        source = package["source"]
        source_url = (
            f"https://static.crates.io/crates/{quote(name)}/{quote(name)}-{quote(version)}.crate"
            if source.startswith("registry+")
            else "{}/archive/{}.tar.gz".format(*git_source(source))
        )
        license_files = []
        paths = notice_candidates(directory)
        if package.get("license_file"):
            declared = directory / package["license_file"]
            if declared.is_file() and declared not in paths:
                paths.append(declared)
        for path in paths:
            relative = path.relative_to(directory).as_posix()
            destination = f"crates/{name}-{version}/{relative}"
            contents = path.read_bytes()
            generated[destination] = contents
            if source.startswith("git+"):
                repository, revision = git_source(source)
                origin_url = (
                    f"{repository}/blob/{revision}/"
                    f"{quote(path.relative_to(git_root(directory)).as_posix())}"
                )
            else:
                origin_url = source_url
            license_files.append({
                "path": destination, "sha256": digest(contents),
                "origin": relative, "source_url": origin_url,
            })
        # Preserve the Rune fork's repository-wide notice for the Rune crates
        # and its patches alongside upstream per-crate licensing.
        if source.startswith("git+"):
            checkout = git_root(directory)
            for path in checkout.iterdir():
                if not path.is_file() or not NOTICE_NAME.search(path.name):
                    continue
                contents = path.read_bytes()
                destination = f"crates/{name}-{version}/repository/{path.name}"
                generated[destination] = contents
                repository, revision = git_source(source)
                license_files.append({
                    "path": destination, "sha256": digest(contents),
                    "origin": f"repository/{path.name}",
                    "source_url": f"{repository}/blob/{revision}/{quote(path.name)}",
                })
        vcs = directory / ".cargo_vcs_info.json"
        upstream_revision = None
        if vcs.exists():
            upstream_revision = json.loads(vcs.read_text(encoding="utf-8"))["git"]["sha1"]
        # Recover workspace-root originals omitted from a published crate.
        for entry in supplemental:
            if (
                (package.get("repository") or "").removesuffix(".git")
                == entry["repository"].removesuffix(".git")
                and upstream_revision == entry.get("applies_to_commit", entry["commit"])
            ):
                contents = (supplemental_root.parent / entry["path"]).read_bytes()
                destination = f"crates/{name}-{version}/upstream/{entry['filename']}"
                generated[destination] = contents
                license_files.append({
                    "path": destination, "sha256": digest(contents),
                    "origin": "upstream/" + entry["filename"], "source_url": entry["url"],
                    "upstream_license_commit": entry["commit"],
                    "applies_to_commit": entry.get("applies_to_commit", entry["commit"]),
                    "provenance_note": entry.get("note"),
                })
        license_texts = [
            item for item in license_files
            if re.search(r"(?i)licen[cs]e|copying|ofl", Path(item["path"]).name)
        ]
        if not license_texts:
            raise ValueError(f"No original license text found: {name} {version}")
        if not any(len(generated[item["path"]]) >= 512 for item in license_texts):
            raise ValueError(
                f"Only short license pointers found; provide the complete original license: "
                f"{name} {version}"
            )
        inventory_packages.append({
            "name": name, "version": version, "source": source,
            "license_expression": expression, "authors": package["authors"],
            "source_url": source_url, "repository": package.get("repository"),
            "crate_checksum": checksums.get((name, version, source)),
            "upstream_commit": upstream_revision,
            "license_files": license_files,
        })

    toolchain = None
    if not args.skip_toolchain:
        version_lines = run(["rustc", "-Vv"]).splitlines()
        version_info = dict(
            line.split(": ", 1) for line in version_lines[1:] if ": " in line
        )
        toolchain = {
            "version": version_info["release"], "commit": version_info["commit-hash"],
            "host": version_info["host"],
            "source_url": f"https://github.com/rust-lang/rust/tree/{version_info['commit-hash']}",
        }
        documentation = Path(run(["rustc", "--print", "sysroot"]).strip()) / "share/doc/rust"
        copyright_file = documentation / "COPYRIGHT-library.html"
        if not copyright_file.is_file():
            raise ValueError("Installed Rust toolchain lacks COPYRIGHT-library.html")
        generated["toolchain/COPYRIGHT-library.html"] = copyright_file.read_bytes()
        for path in sorted((documentation / "licenses").iterdir()):
            if path.is_file():
                generated[f"toolchain/licenses/{path.name}"] = path.read_bytes()

    inventory = {
        "schema_version": 1,
        "target": TARGET, "scope": "normal+build",
        "package_count": len(inventory_packages),
        "cargo_lock_sha256": digest(lock.read_bytes()),
        "cargo_toml_sha256": digest(manifest.read_bytes()),
        "rust_toolchain": toolchain,
        "packages": inventory_packages,
    }
    lines = [
        "# Rust dependency notices", "",
        f"Target: {TARGET}. Packages: **{len(inventory_packages)}**.", "",
        "Generated from the locked, activated Cargo normal/build dependency graph.",
        ("Both RuneTeX binaries share this graph. Build and proc-macro notices are"
         if manifest == REPOSITORY / "Cargo.toml"
         else "The selected package's build and proc-macro notices are"),
        "retained conservatively; this is not a claim that every build tool is linked",
        "into the shipped executables. Development-only and other-platform edges",
        "are excluded. Original license, copyright, AUTHORS and NOTICE files are",
        "preserved below, including additional notices for vendored code and fonts.",
        "The license expressions are the upstream declarations; retaining multiple",
        "alternative license texts does not change an upstream licensing choice.", "",
        "Run python scripts/Generate-RustNotices.py after changing Cargo dependencies.",
        "Run the same command with --check to verify the pinned inputs and outputs.", "",
        "| Package | Declared license | Source | Original notice files |",
        "| --- | --- | --- | --- |",
    ]
    for package in inventory_packages:
        links = "<br>".join(
            f"[{item['origin']}]({quote(item['path'])})" for item in package["license_files"]
        )
        notes = sorted({
            item["provenance_note"] for item in package["license_files"]
            if item.get("provenance_note")
        })
        if notes:
            links += "<br>" + "<br>".join(notes)
        expression = package["license_expression"] or "See original license file"
        lines.append(
            f"| {package['name']} {package['version']} | {expression} | "
            f"[exact source]({package['source_url']}) | {links} |"
        )
    if toolchain:
        lines += [
            "", "## Rust standard library", "",
            f"Rust **{toolchain['version']}**, commit {toolchain['commit']}.",
            "The installed toolchain's original library copyright inventory and",
            "associated license texts cover the standard library and its listed",
            "third-party portions, including compiler-builtins notices.", "",
            "[Library copyright notices](toolchain/COPYRIGHT-library.html) · "
            f"[Exact source]({toolchain['source_url']})",
        ]
    generated["INDEX.md"] = ("\n".join(lines) + "\n").encode("utf-8")
    if output == supplemental_root.parent:
        for path in sorted(supplemental_root.rglob("*")):
            if path.is_file():
                generated[path.relative_to(output).as_posix()] = path.read_bytes()
    inventory["files"] = [
        {"path": relative, "sha256": digest(contents)}
        for relative, contents in sorted(generated.items())
    ]
    generated["inventory.json"] = json_bytes(inventory)
    prior_inventory = output / "inventory.json"
    prior_files: dict[str, str] = {}
    if prior_inventory.is_file():
        previous = json.loads(prior_inventory.read_text(encoding="utf-8"))
        prior_files = {item["path"]: item["sha256"] for item in previous["files"]}
    stale = {
        relative: expected
        for relative, expected in prior_files.items()
        if relative not in generated
    }
    actual_files = {
        path.relative_to(output).as_posix()
        for path in output.rglob("*")
        if path.is_file() or path.is_symlink()
    } if output.exists() else set()
    unexpected = actual_files - set(generated) - set(stale)
    if unexpected:
        raise ValueError(
            "Unmanaged files in notice output (preserved): " + ", ".join(sorted(unexpected))
        )
    for relative, expected in stale.items():
        path = safe_output(output, relative)
        if path.exists() and digest(path.read_bytes()) != expected:
            raise ValueError(f"Modified obsolete notice file was preserved: {relative}")
    if args.check and stale:
        raise ValueError("Obsolete managed notice files: " + ", ".join(sorted(stale)))
    if args.check:
        for relative, contents in generated.items():
            path = safe_output(output, relative)
            if not path.is_file() or path.read_bytes() != contents:
                raise ValueError(f"Missing or stale notice file: {relative}")
        print(f"Verified {len(packages)} Windows Cargo packages; {len(generated)} notice files")
        return
    # Remove only obsolete files listed (and hash-matched) in the old inventory.
    # Unknown or modified files above are always preserved; no recursive deletion.
    for relative in stale:
        safe_output(output, relative).unlink(missing_ok=True)
    for relative, contents in generated.items():
        path = safe_output(output, relative)
        path.parent.mkdir(parents=True, exist_ok=True)
        if not path.exists() or path.read_bytes() != contents:
            path.write_bytes(contents)
    print(f"Generated {len(packages)} Windows Cargo packages; {len(generated)} notice files")


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, KeyError, subprocess.CalledProcessError) as error:
        print(f"Rust notice generation failed: {error}", file=sys.stderr)
        if isinstance(error, subprocess.CalledProcessError) and error.stderr:
            print(error.stderr, file=sys.stderr)
        sys.exit(1)
