# RuneTeX bundled component sources

These links identify the original sources of the components in the matching
Windows x64 RuneTeX installer. Full notices, package identities, checksums,
build recipes and patches are included in the installer under `licenses/`.
The matching source-download directions are also published in the GitHub
Release body.

The repository stores the originals in `licenses/notices.zip` with its
SHA-256 in `licenses/notices.sha256`. Build tools expand that archive into
`.cache/notices/` and preserve the original `licenses/` paths in the installer.

## texlab 5.26.0

- [Original source ZIP at release commit 45c3ced6790b508a12a2aab1938e58023173017e](https://codeload.github.com/latex-lsp/texlab/zip/45c3ced6790b508a12a2aab1938e58023173017e)
- [Browse the matching source](https://github.com/latex-lsp/texlab/tree/45c3ced6790b508a12a2aab1938e58023173017e)
- [Matching Cargo.lock](https://github.com/latex-lsp/texlab/blob/45c3ced6790b508a12a2aab1938e58023173017e/Cargo.lock)
- [Original Windows release build workflow](https://github.com/latex-lsp/texlab/blob/45c3ced6790b508a12a2aab1938e58023173017e/.github/workflows/publish.yml)
- [Source build instructions](https://github.com/latex-lsp/texlab/blob/45c3ced6790b508a12a2aab1938e58023173017e/README.md#building-from-source)

The unmodified official Windows x64 binary is bundled. The source ZIP's
SHA-256 is `cfa42f664a47b6de3d26539c46b662f94a8938c8734f5a36412139949249f430`.
Build with `cargo build --target x86_64-pc-windows-msvc --all-features --release --locked`.
The lockfile identifies the dependency versions used by Cargo. The license
inventory includes the matching workflow, lockfile, dependency source URLs and
copyright/license notices under `licenses/texlab/`.

## Poppler and poppler-data

- [Poppler 26.09.0 original source](https://poppler.freedesktop.org/poppler-26.09.0.tar.xz)
- [Windows Poppler recipe and patches, commit 13d784d77510e73d6066a75a79cd8e6040a6a261](https://github.com/conda-forge/poppler-feedstock/tree/13d784d77510e73d6066a75a79cd8e6040a6a261)
- [poppler-data 0.4.12 original source](https://poppler.freedesktop.org/poppler-data-0.4.12.tar.gz)
- [Windows bundle packaging source, commit dc4cae5410474e4d75c5c494eb1a8c6774e47457](https://github.com/oschwartz10612/poppler-windows/tree/dc4cae5410474e4d75c5c494eb1a8c6774e47457)
- [Original binary release](https://github.com/oschwartz10612/poppler-windows/releases/tag/v26.09.0-0)
- [Original package build record](https://github.com/oschwartz10612/poppler-windows/actions/runs/34991014991)

The Poppler binary package is `poppler-26.09.0-h924501e_0`. Exact sources for
its bundled native dependencies are listed per package in
`licenses/poppler/INDEX.md` and `inventory.json` in the installed notices.
That directory also preserves the original package license texts, recipe
sources and patches. Individual native components, including Microsoft
redistributable runtimes, retain their respective license terms.

## Rune, Rust dependencies and fonts

- [Rune source at a05683be96cbd725ca1fbb34810368309ac534a5](https://github.com/anomalyco/rune/tree/a05683be96cbd725ca1fbb34810368309ac534a5)
- [Rune source archive](https://github.com/anomalyco/rune/archive/a05683be96cbd725ca1fbb34810368309ac534a5.tar.gz)
- [Embedded Inter font files](https://github.com/anomalyco/rune/tree/a05683be96cbd725ca1fbb34810368309ac534a5/crates/rune/assets)

`licenses/rust/INDEX.md` and `inventory.json` in the installed notices identify
the Windows dependency graph and link each crate to its exact source archive
or immutable Git revision. The Rust toolchain version/source and standard
library notices are included there. `licenses/fonts/` identifies the three
embedded Inter fonts and preserves their SIL Open Font License.

## Release publishers and mirrors

Publish the version-matched source-download directions in the GitHub Release
body. `scripts/Write-ReleaseNotes.ps1` prepares that body from the changelog
and the exact source inventories. Maintain the source availability required
by the applicable component licenses. If an upstream source location becomes
unavailable, preserve an accessible copy of the identified corresponding
source and update the download directions.
