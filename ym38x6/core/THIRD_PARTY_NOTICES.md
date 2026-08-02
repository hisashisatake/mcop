# Third-Party Notices

このクレート（`ym38x6-core`）には、以下のサードパーティ製ソフトウェアの一部を
参考にして移植したコードが含まれています。

## ymfm

- リポジトリ: https://github.com/aaronsgiles/ymfm
- ライセンス: BSD 3-Clause License
- 参照箇所: `src/ymfm_fm.ipp` の `s_algorithm_ops` テーブル
  （`fm_channel<RegisterType>::output_4op` が参照するOPN系4opアルゴリズム定義）
- 参照時点のコミットハッシュ: `17decfae857b92ab55fbb30ade2287ace095a381`（mainブランチ）
- 移植先: `src/algorithm.rs` の `ALGORITHMS`（8アルゴリズムの結線テーブル）

OPQ(YM3806)はOPN/OPM系チップと同一のFMコアを共有しており、8アルゴリズムの
結線テーブルも共通と考えられるため、OPN系の定義を38x6エンジンのアルゴリズム
結線として採用した。

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
