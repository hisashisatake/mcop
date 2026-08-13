# opzref — ymfm OPZ 参照レンダラ（op505向け）

TX81Z（YM2414 / OPZ）の `.syx` ボイスを **ymfm エミュレータで直接レンダリング**し、
WAV 化する検証用ツール。`opz2op505` の変換忠実度を、実機録音の交絡（残響・ポリフォニー・
velocity 不明）なしに突き合わせるための「実チップに近い参照音源」を作る。

由来: `ym38x6/tools/opzref4x6`（コミット b61ba7a 時点の複製、2026-08-13）。デフォーク後の
op505ツール群向け複製（fork-on-write）。ym38x6/tools/opzref4x6側の修正は自動では
反映されない。

ymfm 本体（BSD 3-Clause）は `vendor/ymfm/` に同梱し、`cc` クレートで C++ をビルドして
FFI（`csrc/shim.cpp`）で呼ぶ。ライセンスは [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) 参照。

## 使い方

```powershell
# ツールチェーン自己テスト（単一キャリアのサイン波を出力）
cargo run -p opzref --release -- --selftest out.wav

# .syx の voice 0 を G#4(midi68) で 2.5 秒レンダリング
cargo run -p opzref --release -- render "bank.syx" 0 out.wav --note 68 --dur 2.5 --gate 2.0
```

### render オプション

- `--note <midi>`  発音ノート（既定 68 = G#4）
- `--dur <sec>`    総レンダリング秒数（既定 2.5）
- `--gate <sec>`   キーオン保持秒数（既定 2.0、以降はリリース）
- `--kc <hex>`     OPM キーコードを直接指定（音程キャリブレーション用）
- `--slots a,b,c,d` `ops[0..3]`（=[OP4,OP3,OP2,OP1]）を割り当てるレジスタ slot。
  既定 `0,2,1,3` は VMEM のバイト配置（OP4@0,OP2@10,OP3@20,OP1@30）がそのまま
  物理slot順を反映しているという前提（TX81Z公式ドキュメント・実測波形で確認済み）。

## メモ

- マスタークロック 3.579545 MHz → ネイティブ出力 55930 Hz。
- レジスタ計算ロジックは `src/regs.rs`（`RegSink`トレイトで実機/テスト双方から共有）に
  分離してあり、`tests/golden/reg_sweep.fnv` で回帰を検出する。
- ym38x6版（`opzref4x6`）との出力バイト一致を移植時に確認済み（WAVハッシュ一致、
  通常ボイス・force-sine・カスタムslots・selftestとも）。
- OPZ は ymfm 上でも一部「未解明」と注記されており、参照も完全ではないが実機に最も近い。
