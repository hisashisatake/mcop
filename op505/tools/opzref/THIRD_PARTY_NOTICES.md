# Third-Party Notices

このツール（`opzref`、op505向け）は、以下のサードパーティ製ソフトウェアのソースコードを
**そのまま同梱（vendoring）** して使用しています。

## ymfm

- リポジトリ: https://github.com/aaronsgiles/ymfm
- ライセンス: BSD 3-Clause License（`vendor/ymfm/LICENSE` に全文）
- 同梱箇所: `vendor/ymfm/`
  - `ymfm.h` / `ymfm_fm.h` / `ymfm_fm.ipp` / `ymfm_opz.h` / `ymfm_opz.cpp`
  - YM2414（OPZ）エミュレーションに必要な最小ファイルセット
- 取得元: main ブランチの raw ソース（2026-06-20 取得）
- 用途: TX81Z(OPZ) ボイスを実チップに近い形でレンダリングする参照音源。
  `csrc/shim.cpp` から `ymfm::ym2414` を呼び出す。

各ソースファイル先頭に BSD 3-Clause のライセンス全文が含まれており、
`vendor/ymfm/LICENSE` にも同一のライセンス全文を同梱している。

### License Text

```
BSD 3-Clause License

Copyright (c) 2021, Aaron Giles

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are met:

1. Redistributions of source code must retain the above copyright notice, this
   list of conditions and the following disclaimer.

2. Redistributions in binary form must reproduce the above copyright notice,
   this list of conditions and the following disclaimer in the documentation
   and/or other materials provided with the distribution.

3. Neither the name of the copyright holder nor the names of its
   contributors may be used to endorse or promote products derived from
   this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE
FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER
CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY,
OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```
