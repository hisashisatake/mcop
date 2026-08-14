# opzref4x6 — ymfm OPZ 参照レンダラ

TX81Z（YM2414 / OPZ）の `.syx` ボイスを **ymfm エミュレータで直接レンダリング**し、
WAV 化する検証用ツール。`opz2x6` + `ym38x6` の変換・エンジン忠実度を、
実機録音の交絡（残響・ポリフォニー・velocity 不明）なしに突き合わせるための
「実チップに近い参照音源」を作る。

ymfm 本体（BSD 3-Clause）は `vendor/ymfm/` に同梱し、`cc` クレートで C++ をビルドして
FFI（`csrc/shim.cpp`）で呼ぶ。ライセンスは [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) 参照。

## 使い方

```powershell
# ツールチェーン自己テスト（単一キャリアのサイン波を出力）
cargo run -p opzref4x6 --release -- --selftest out.wav

# .syx の voice 0 を G#4(midi68) で 2.5 秒レンダリング
cargo run -p opzref4x6 --release -- render "bank.syx" 0 out.wav --note 68 --dur 2.5 --gate 2.0
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
- 解析は `tools/opz2x6/private/investigate/harmonics_table.py`（H1基準dB表）と併用。
- OPZ は ymfm 上でも一部「未解明」と注記されており、参照も完全ではないが実機に最も近い。
