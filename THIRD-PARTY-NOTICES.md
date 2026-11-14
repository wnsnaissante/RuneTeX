# Third-party notices

RuneTeX's own code is covered by the MIT license in `LICENSE`. Each bundled
component retains its own copyright and license. The original license and
copyright texts are installed in the `licenses/` directory alongside the app.
The repository preserves those originals and their build recipes in
[notices.zip](licenses/notices.zip), verified by
[notices.sha256](licenses/notices.sha256). Build tools expand the archive into
an ignored local cache. The readable [source index](licenses/SOURCES.md) is
kept beside the archive.

## Included components and source downloads

| Component | Bundled version | Original sources and build information | License texts and inventory |
| --- | --- | --- | --- |
| texlab | 5.26.0; commit `45c3ced6790b508a12a2aab1938e58023173017e` | [Exact source ZIP](https://codeload.github.com/latex-lsp/texlab/zip/45c3ced6790b508a12a2aab1938e58023173017e), [release build workflow](https://github.com/latex-lsp/texlab/blob/45c3ced6790b508a12a2aab1938e58023173017e/.github/workflows/publish.yml) | `licenses/texlab/`: GPL text, matching Cargo.lock and dependency notices |
| Poppler | 26.09.0, conda build `h924501e_0` | [Original source archive](https://poppler.freedesktop.org/poppler-26.09.0.tar.xz), [Windows recipe and patches](https://github.com/conda-forge/poppler-feedstock/tree/13d784d77510e73d6066a75a79cd8e6040a6a261) | `licenses/poppler/` |
| poppler-data | 0.4.12 | [Original source archive](https://poppler.freedesktop.org/poppler-data-0.4.12.tar.gz) | Native inventory and the original `tools/poppler/share/poppler/COPYING*` files |
| Poppler's bundled native libraries and Microsoft runtimes | Exact versions and build IDs in native inventory | Original source URLs, package archives, recipes and patches in each package record | `licenses/poppler/` |
| Rune and compiled Rust dependencies | Windows normal/build dependency graph recorded against Cargo.lock | Versioned crate source downloads and immutable Git revisions | `licenses/rust/` |
| Rust standard library | Toolchain recorded in `rust-toolchain.toml` | Exact toolchain version and source links in Rust inventory | Rust toolchain copyright/license texts under `licenses/rust/toolchain/` |
| Inter Regular, Medium and SemiBold | 4.001; git `9221beed3` | [Font files in the pinned Rune commit](https://github.com/anomalyco/rune/tree/a05683be96cbd725ca1fbb34810368309ac534a5/crates/rune/assets) | `licenses/fonts/`, SIL Open Font License 1.1 |

## texlab

texlab is developed by Eric Förster, Patrick Förster and texlab contributors.
It runs as a separate Language Server Protocol executable. The bundled EXE is
an unmodified copy of the official Windows x64 release. Its source ZIP is
pinned to the release commit, rather than a moving branch. The matching
upstream build workflow, Cargo.lock, source-archive checksums and dependency
source locations are provided in `licenses/texlab/`.

## Poppler and native packages

The unmodified Windows binaries come from
[poppler-windows v26.09.0-0](https://github.com/oschwartz10612/poppler-windows/releases/tag/v26.09.0-0).
The original package workflow copied DLLs from its resolved conda environment.
The native inventory identifies those package versions and build IDs using
[the original build record](https://github.com/oschwartz10612/poppler-windows/actions/runs/34991014991)
and checks the distributed files against the original package/source hashes.
The corresponding package license texts, rendered build recipes, patches and
source locations are preserved under `licenses/poppler/`.

Poppler's code, poppler-data and each native dependency have their own terms;
the poppler-data COPYING files do not replace the other components' notices.
Microsoft runtime files retain the Microsoft license terms included with
their identified redistributable packages.

## Fonts and separately installed software

The native Windows application embeds the three Inter fonts listed above.
Other UI/editor fonts are loaded from the user's system and are not copied
into this installer. MiKTeX and TeX Live are separately installed software;
their binaries and TeX packages are not included in this installer.

## Release materials

`licenses/SOURCES.md` is the installed source-download index. These notices,
the full license texts and the versioned inventories are included in the
installer. The build script produces the setup executable and its checksum.

`scripts/Write-ReleaseNotes.ps1` prepares the GitHub Release body using the
version's changelog and the inventories' exact source/build links. Publish
those directions in the release body and retain clear access to the identified
corresponding sources when distributing or mirroring the release.

The installer design review used Damir Sokolovsky's
[unslop guidelines](https://github.com/heydamir/unslop), licensed CC BY 4.0.
