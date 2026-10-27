# Third-party notices

The daemon and desktop app have separate Cargo dependency graphs. A list of
SPDX identifiers is not a binary-distribution notice: the original copyright,
permission and additional NOTICE text must travel with the binaries too.

## Generate and verify

From the repository root, with Python 3.9+ and `cargo-about` installed
(the tested collector is `cargo-about 0.8.4`):

```sh
cargo install cargo-about --version 0.8.4 --locked
cargo fetch --locked
cargo fetch --locked --manifest-path crates/melba/Cargo.toml
./ci/licenses.sh
python3 packaging/licenses.py --check target/third-party
```

The output directory contains a plain-text notice bundle for each binary and a
receipt binding both bundles to the source manifests, lockfiles and licensing
inputs. Collection is offline and locked; only the explicit fetch steps use the
network. The installer places these documents under
`<Cargo installation root>/share/licenses/melba/`; the Arch package uses
`/usr/share/licenses/melba-git/`. Both routes check freshness and integrity before
staging them, including when repackaging with tests disabled.

[`packaging/licenses/`](packaging/licenses/) owns the accepted SPDX choices and
reviewed supplements. Cargo-about resolves the Linux default-feature dependency
graphs, including build dependencies but not dev-only dependencies. The
allowlist currently includes CC0-1.0; it is not a CC0 prohibition. GPL/AGPL
choices are not accepted; a permitted alternative in an OR expression can be
selected. MPL components remain MPL-licensed, and their exact-version
source download URLs are included in each bundle. Their licensing does not
relicense the project's own code.

The generator retains original source notices in addition to selected license
texts. Local subcrates inherit the source root's shared notices with or without
Git metadata, so source archives and Git checkouts carry the same attribution.
It does not accept a synthesized template as a substitute for a missing
original notice. A crate whose published archive omits its notice needs a
version-specific supplement with an immutable upstream URL and SHA-256. Review
and update those inputs when changing dependencies; do not silence a collection
failure. When upstream grants its license by reference in a README rather
than a separate license file, retain that original, pinned grant alongside
the full selected license terms; do not invent a copyright notice.
Embedded fonts also have their own conditions, including the Hack
font's Bitstream Vera notice, which is not fully described by the crate's SPDX
metadata.

## Code and assets carried here

The project root is dual MIT / Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`).
The desktop app (`crates/melba`) is MIT-only and descends from Carmine
Paolino's Spotifast; its original copyright and permission notice remains in
`crates/melba/LICENSE` and is shipped in portable archives as
`SPOTIFAST-LICENSE`. The root's dual-license option does not relicense it. The Chromium CDM interface
headers retain their BSD notice under `crates/melba-cdm/third_party/cdm-api/`.
Font and icon notices live beside the assets. The generated bundles include
these notices without replacing or editing their original terms. Portable
archives carry `NOTICE.md`, both project licences, the Spotifast notice, the
generated per-binary notice bundles and their receipt; packaging refuses to
build an archive when those inputs are missing.

The Git dependencies, including fastframe and the egui/winit forks, belong to
the Cargo dependency graphs above. Their resolved identities and original
notices are collected in the generated bundles; this document is not a
hand-maintained dependency inventory.

## Runtime components and research materials

Source builds of the Rust executables do not embed Widevine, the Apple Music
APK, or a wrapper rootfs. Widevine is loaded from the user's existing
installation rather than downloaded or redistributed by Melba. A wrapper
runtime bundle (including any rootfs with Apple or Android shared libraries) is
not part of this source distribution; do not include it in a redistributed
source archive, package, image or build. The helper's source license, the
rootfs libraries' terms, and Melba's licenses are separate. Pins and hashes
verify adopted bytes, not licensing or permission to redistribute them.
Establish the relevant grants before using or redistributing any runtime.
A subscription and local downloading do not by themselves establish those
grants.

Screenshots, fixtures and research outputs require content-specific review.
The [documentation-image policy](evidence/README.md) identifies the synthetic
replacement set. The [material-provenance record](materials/README.md) binds
its declared fixture and extracted-string subset to byte identities, sources
and explicit unresolved questions. Neither record is a blanket publication
allowlist. The project's MIT/Apache licenses do not grant rights to third-party
content that its licensors do not control. Review the applicable grants or
other grounds before publication; deleting a current file does not remove it
from Git history.

Separately installed system libraries also retain their own terms. The Cargo
notice gate checks its declared dependency policy and collected notices, not
image rights, extracted material, signing-key provenance, runtime bundle
licensing, or legal clearance for the repository as a whole.
