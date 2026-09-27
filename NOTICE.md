# Notices

leafcutter-ant is a port of [Dash](https://github.com/bridge-core/dash-compiler)
0.13.0 and is released under Dash's MIT licence, in `LICENSE`. It also ports
code from, and vendors test data from, the projects below. Each is released
under the MIT License, whose text follows the table, with the copyright line
given here.

| What | From | Copyright |
| --- | --- | --- |
| `crates/leafcutter-ant/src/json/json5.rs` is a port of `lib/parse.js`, and `json5_unicode.rs` is generated from its character tables | [json5](https://github.com/json5/json5) 2.2.1 | Copyright (c) 2012-2018 Aseem Kishore, and [others] |
| `crates/leafcutter-ant/src/pathe.rs` | [pathe](https://github.com/unjs/pathe) 2.0.2 and 1.1.2 | Copyright (c) Pooya Parsa <pooya@pi0.io> - Daniel Roe <daniel@roe.dev> |
| `crates/leafcutter-ant/src/glob/picomatch.rs`, through the copy vendored in [@bridge-editor/common-utils](https://github.com/bridge-core/common-utils) 0.3.3 | [picomatch](https://github.com/micromatch/picomatch) | Copyright (c) 2017-present, Jon Schlinkert; common-utils: Copyright (c) 2021 bridge-team |
| `is_glob` and `is_extglob` in `crates/leafcutter-ant/src/glob/mod.rs` | [is-glob](https://github.com/micromatch/is-glob) 4.0.3 and [is-extglob](https://github.com/micromatch/is-extglob) 2.1.1 | Copyright (c) 2014-2017, Jon Schlinkert; Copyright (c) 2014-2016, Jon Schlinkert |
| `crates/leafcutter-ant/tests/data/fileDefinitions.json` and `packDefinitions.json`, unchanged | [bridge-core/editor-packages](https://github.com/bridge-core/editor-packages) at `10e360dc24194651b814994467ebdb8f0403c62e` | Copyright (c) 2021 bridge-team |

## MIT License

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

[others]: https://github.com/json5/json5/contributors
