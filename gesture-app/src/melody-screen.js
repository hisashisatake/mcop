// メロディ画面（フェーズ6で実装予定）。フェーズ3時点ではプレースホルダーのみ。

export function setupMelodyScreen(canvas) {
  return { draw: (ctx) => draw(ctx, canvas) };
}

function draw(ctx, canvas) {
  const W = canvas.width;
  const H = canvas.height;
  ctx.fillStyle = '#111';
  ctx.fillRect(0, 0, W, H);
  ctx.textAlign = 'center';
  ctx.fillStyle = '#555';
  ctx.font = '20px monospace';
  ctx.fillText('メロディ画面（準備中）', W / 2, H / 2);
}
