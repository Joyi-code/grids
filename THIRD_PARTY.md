# Third-party code and assets

Grids is MIT licensed (see `LICENSE`). It includes or adapts the following.

## quiet-md (MIT)

Copyright (c) 2026 s1dashu. https://github.com/s1dashu/quiet-md

Adapted: the theme tokens and dark-mode colour adaptation (`src/theme.rs`),
the shared UI pieces such as tooltips, popover shell, menu rows, title bar
behaviour, window controls and single-line text with ellipsis (`src/ui.rs`),
the local GPUI / taffy / xattr patches under `vendor/` (see
`vendor/PATCHES.md`), the workspace clippy settings and `rust-toolchain.toml`.
quiet-md's icons and app name are not used.

```
MIT License

Copyright (c) 2026 s1dashu

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## GPUI (Apache-2.0)

Copyright Zed Industries, Inc. https://github.com/zed-industries/zed

The UI framework, used as the crates.io `gpui` 0.2.2 with local patches in
`vendor/gpui` (licence text in `vendor/gpui/LICENSE-APACHE`). The single-line
text field in `src/input.rs` is adapted from GPUI's `examples/input.rs`
(Apache-2.0); changes: Grids key context and bindings, placeholder, submit /
cancel events, horizontal scrolling and selection helpers.

## taffy (MIT) and xattr (MIT / Apache-2.0)

Layout engine and extended-attribute crate used by GPUI, with local patches
in `vendor/taffy` and `vendor/xattr`; their licences are in those folders.

## Lucide icons (ISC)

https://lucide.dev — the SVG icons in `assets/icons/` are Lucide 0.468.0, with
the stroke width set to 1.75. `folder-fill.svg` and `star-fill.svg` are filled
versions of Lucide's `folder` and `star`.

```
ISC License

Copyright (c) for portions of Lucide are held by Cole Bemis 2013-2022 as part
of Feather (MIT). All other copyright (c) for Lucide are held by Lucide
Contributors 2022.

Permission to use, copy, modify, and/or distribute this software for any
purpose with or without fee is hereby granted, provided that the above
copyright notice and this permission notice appear in all copies.

THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN
ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF
OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

Other Rust dependencies are pulled from crates.io under their own licences
(listed in `Cargo.lock`).
