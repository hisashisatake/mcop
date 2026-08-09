# Third-Party Notices

このクレート（`sound-core`）には、以下のサードパーティ製ソフトウェアの一部を
参考にして移植したコードが含まれています。

## Andrew Simper (Cytomic) SVF

- 出典: Andrew Simper (Cytomic), "SvfLinearTrapOptimised2.pdf"
  （Solving the continuous SVF equations using a trapezoidal integrator）
  https://cytomic.com/files/dsp/SvfLinearTrapOptimised2.pdf
- ライセンス: 公開リポジトリのコードではなく技術文書中の参考実装のため、明確な
  ライセンス表記は無い（Surge・Vital等のOSS音源でも同様に著者名のみのクレジット
  で実装されている）
- 参照箇所: 文書中のTPT(Topology-Preserving Transform)型SVFの式
  （ic1eq/ic2eq、g/k/a1/a2/a3、v1/v2/v3によるLP/HP/BP導出）
- 移植先: `src/vcf.rs` の `Svf::process`
