# License policy

Chiptune Studio's original code and bundled example compositions are **MIT**.

Dependencies must be available under permissive terms compatible with distributing this MIT project. No dependency is selected under GPL, LGPL, or AGPL terms. When upstream offers several licenses, an explicitly documented permissive option is selected. This does not relicense upstream code as MIT or remove its attribution requirements.

The pinned inventory is in `licenses/inventory.json`. `licenses/THIRD_PARTY.txt` preserves upstream license and attribution notices, including those required when distributing the executable. Ship that file with binary distributions.

## Ogg Vorbis

`vorbis_rs`, `aotuv_lancer_vorbis_sys`, and `ogg_next_sys` are BSD-3-Clause. Their bundled libvorbis/aoTuV and libogg code uses permissive BSD notices, preserved in the notice bundle. The wrapper's workspace license and the additional permissive LPC source attribution to Jutta Degener, Carsten Bormann, and Technische Universitaet Berlin are also preserved. The encoder is linked into the application; no GPL command-line encoding utility is used.

Sources checked on 2026-10-22:

- [Xiph licensing FAQ](https://www.xiph.org/vorbis/faq/).
- [Vorbis Rust bindings license](https://github.com/ComunidadAylas/vorbis-rs/blob/master/LICENSE).
- The exact pinned crates' `Cargo.toml`, vendored `COPYING` files, and build scripts.

## Alternative upstream licenses

Most Rust dependencies offer MIT or Apache-2.0; MIT is selected where offered. `hound` uses Apache-2.0. The `unicode-ident` package also requires the Unicode-3.0 notice. The target-specific `r-efi` package offers MIT, Apache-2.0, or LGPL-2.1-or-later; **MIT is selected**, with its upstream `AUTHORS` notice preserved. LGPL is not the selected license.

## Updating dependencies

When changing `Cargo.lock`, review every new or updated dependency, including target-specific and build dependencies, bundled native sources, and their notices. Refresh the inventory, its normalized Cargo.lock hash, and the full notice bundle. Record the selected permissive license for alternatives.

Local `license_policy` tests reject an inventory that does not match the lockfile, missing package coverage, or an unreviewed license expression. New expressions require explicit review, even if they appear permissive. Run these checks locally; do not introduce GitHub CI.
