// コード画面（フロー方式）：現在のコードを中心に、左へ過去コードの履歴、右へ「記録済みの続き
// （未来）」と「次の候補」を並べる。
//
// 過去/未来のナビゲーションと、Ctrl+Z/Ctrl+Yの取り消しは別の概念として扱う:
//   - 過去/未来コードのクリックは「再生位置の移動」（cursorを動かすだけ、entriesは不変）
//   - Ctrl+Z/Ctrl+Yは「新しいコードを選択した」という編集操作そのものの一般的なUndo/Redo
//     （過去へ戻ってから新しいコードを選んで先の履歴を上書きした場合、Ctrl+Zはその上書き
//     操作自体を取り消し、上書き前のentries全体（上書きされる前に見えていた続きも含む）へ
//     丸ごと復元する。例: A-B-C-Dと選んでからBまで戻りEを選ぶとA-B-Eになるが、Ctrl+Zを押すと
//     A-B-C-D（カーソルはB）に戻る）。編集操作でないただのカーソル移動（過去/未来クリックや
//     Ctrl+Z/Y自体）はこのUndo/Redoスタックに積まない
//
// 操作:
//   候補セルをクリック … 押している間だけ発音し、離しても選択は確定して現在コードになる
//                        （セル内上下でベロシティ、上側ほど強い）。cursorより先の履歴が
//                        あれば、この時点で破棄される（Ctrl+Zで丸ごと復元可能）
//   過去/未来コードをクリック … 押している間だけ発音し、その地点まで再生位置を移動する
//                        （過去=戻る、未来=記録済みの続きへ進む。どちらも即座に確定）
//   Ctrl+Z             … 直前のコード選択（上書きを含む）を取り消す（発音はしない）
//   Ctrl+Y/Ctrl+Shift+Z … Ctrl+Zで取り消した選択をやり直す（発音はしない）
//   Shift               … 4和音中心のレイヤーへ切替（II-V-Iが縦2行以内に収まる）
//   Ctrl                … sus・付加音系のレイヤーへ切替
//   Ctrl+Shift          … aug・オルタード系のレイヤーへ切替
//   ↓/↑                … 候補の行数（3〜12）を増減
//   ←/→                … 候補の列数（1〜3）を増減
//
// コード補助機能:
//   直前に鳴らしたコードを起点に、次に相性の良いコードを緑（定番）/黄（用例は少ないが理論的
//   裏付けあり）/灰（理論スコア低めの自由枠）の3列で提示する（chord-flow.js/theory.js参照）。
//   ピボットコード（近親調との共通コード）には青を混ぜ、それを押した次に転調先固有のコードを
//   押すと調が確定する。
//
// 履歴は線形（cursor+entries配列）。過去/未来への移動はentriesを一切変更しないが、戻った
// 状態で新しいコードを選ぶと、その先の履歴（未来スロットに見えていた続き）は破棄される
// （分岐は保持しない設計判断。詳細はplan「gesture-app コード画面をグリッド方式からフロー方式へ刷新」）。

import { CHORD_CHANNEL, noteOn, noteOff, allNotesOff } from './midi.js';
import { applyTo as applyLfoTo } from './performance-lfo.js';
import { DEFAULT_TONIC_MIDI, NOTE_NAMES, velocityFromCellY, VELOCITY_MIN, VELOCITY_MAX } from './chords.js';
import { pivotKeysFor, confirmsModulation } from './theory.js';
import { computeCandidateGrid, createHistory, keyAt, currentEntry, pendingPivotAt, selectChord, jumpTo } from './chord-flow.js';
import { computePastSlotGeoms } from './chord-layout.js';
import { isActive, onScreenChange } from './screens.js';

const TOP_MARGIN = 40; // 上部の余白（画面タブ・ヒント・ログ等はハンバーガーメニューのドロワーへ移動済みのため最小限でよい）
const BOTTOM_MARGIN = 180; // 左下固定の#hud（コード名の大きな表示）・右下固定の#status-panel（波形メモリ/Bank・Program/Key/TAPテンポ）と過去/現在/未来スロット・候補ブロックが重ならないための余白
const MAX_SCORE_FOR_SHADING = 1.3; // だいたいの上限。alpha計算のクランプ用
const SLIDE_DURATION_MS = 220;
const RIGHT_MARGIN = 24; // 候補ブロックと画面右端の余白（候補ブロックは右端寄せにして過去領域を広げる）
const CURRENT_SIZE = 96; // 現在コードスロットの一辺

// 未来列（記録済みの続き）は従来どおり固定サイズ・固定個数。過去列だけ遠近法的に縮小する
// （[[project_gesture_app_3screen_minidaw_redesign]]参照、進む先は長くならないため対称性より実態を優先）。
const MAX_FUTURE_SLOTS = 4;
const FUTURE_SLOT_SIZE = 84;
const FUTURE_SLOT_GAP = 70;

// 過去列: 遠いほどサイズ・間隔とも指数的に縮む（chord-layout.jsのcomputePastSlotGeoms）
const PAST_BASE_SIZE = 64;
const PAST_MIN_SIZE = 20; // タスクトレイアイコン相当（最も古いスロットの下限サイズ）
const PAST_SHRINK = 0.8;
const PAST_GAP_RATIO = 0.125;
const PAST_LEFT_MARGIN = 16;
const PAST_HIT_MIN_SIZE = 28; // 当たり判定の下限（見た目より少し広く取り極小スロットもクリックできるようにする）
const PAST_ALPHA_MIN = 0.3;
const PAST_ALPHA_BASE = 0.8;
const PAST_ALPHA_DECAY = 0.93;
const PAST_LABEL_FULL_SIZE = 44; // これ以上のサイズならフル名（Cmaj9）
const PAST_LABEL_ROOT_SIZE = 28; // これ以上ならルート音のみ（C）。それ未満は文字なし
const HOVER_EXPAND_SIZE = 72; // 過去スロットにホバーしたときの拡大サイズ（Dock風）
const HOVER_EXPAND_MS = 120;

// 選択時のベロシティをスロット下端からの紺色バーで可視化する（過去・未来・現在の全スロット共通）。
// 再選択（過去/未来クリックでの再生位置移動）ではこの値を書き換えない — entryは選択時に確定した
// ベロシティを保持したまま、発音だけは常に一定のベロシティで行う（cellFromPointが過去/未来に
// yRatio=0.5固定を返すため自然にそうなる）。
const VELOCITY_BAR_COLOR = '40, 70, 150'; // 紺色
const VELOCITY_BAR_ALPHA_SCALE = 0.55; // ラベル文字の可読性を保つため、スロットのalphaより少し抑える

const MIN_ROWS = 3;
const MAX_ROWS = 12;
const MIN_COLS = 1;
const MAX_COLS = 3;
const DEFAULT_ROWS = 5;
const DEFAULT_COLS = 3;

let tonicMidi = DEFAULT_TONIC_MIDI;
let mode = 'major'; // 'major' | 'minor'
let assistRows = DEFAULT_ROWS;
let assistCols = DEFAULT_COLS;

let shiftHeld = false;
let ctrlHeld = false;
let hoverCandidate = null; // {col, row, yRatio}
let hoverSlot = null; // { kind: 'past'|'future', index, yRatio, startTime }（過去・未来のホバー共通。startTimeは過去スロットのDock風拡大アニメーション用）
let sounding = []; // 発音中のノート番号
let pointerHeld = false; // マウスボタンを押している最中か（awaitを跨ぐ取りこぼし対策）

// { entries: [{chord, key:{tonicMidi,mode}, pendingPivot, velocity}], cursor, initialKey }
// velocityは選択時のセル内クリック位置から一度だけ決まり、以後は変化しない（表示用のベロシティ
// バーに使う。過去/未来クリックでの再訪や発音そのものには使わない）。
let history = createHistory({ tonicMidi, mode });
let candidateCache = null; // { cacheKey, grid: [...] }

// Ctrl+Z/Ctrl+Yの編集Undo/Redo用スタック（Mementoパターン）。要素はcommitSelection直前のhistory
// スナップショットそのもの（historyは常に新しいオブジェクトを返す設計のため、参照を保持するだけで
// 安全に巻き戻せる）。過去/未来クリックによるcursor移動はここへ積まない（編集操作ではないため）。
let undoStack = [];
let redoStack = [];

let tonicSelectEl = null;
let modeSelectEl = null;

// 選択・Undo/Redo時の横スライド演出用（純粋に見た目だけの補間。ロジック上は瞬時に切り替わる）
let slideDirection = 0; // +1 = 前進（右→左へ流れる）, -1 = 後退
let slideStart = 0;

/** 発音中チャンネル（performance-lfoが即時反映に使う）。 */
export function activeChannels() {
  return sounding.length > 0 ? [CHORD_CHANNEL] : [];
}

function currentKeyObj() {
  const k = keyAt(history);
  return { tonicPc: ((k.tonicMidi % 12) + 12) % 12, mode: k.mode };
}

function invalidateCandidates() {
  candidateCache = null;
}

function syncControlsFromState() {
  const k = keyAt(history);
  if (tonicSelectEl) tonicSelectEl.value = String(((k.tonicMidi % 12) + 12) % 12);
  if (modeSelectEl) modeSelectEl.value = k.mode;
}

function resetHistory() {
  history = createHistory({ tonicMidi, mode });
  undoStack = [];
  redoStack = [];
  invalidateCandidates();
  syncControlsFromState();
}

function startSlide(direction) {
  slideDirection = direction;
  slideStart = performance.now();
}

function cellFromPoint(canvas, px, py) {
  const layout = computeLayout(canvas);
  const x = px;
  const y = py - TOP_MARGIN;
  if (y < 0) return null;

  // 候補ブロック（縦方向は中央揃えなので、y=0(TOP_MARGIN)ではなくcandidateOriginYを基準にする）
  if (x >= layout.candidateX) {
    const localX = x - layout.candidateX;
    const localY = py - layout.candidateOriginY;
    const col = Math.floor(localX / layout.cellW);
    const row = Math.floor(localY / layout.cellH);
    if (col < 0 || col >= assistCols || row < 0 || row >= assistRows) return null;
    return { kind: 'candidate', col, row, yRatio: (localY - row * layout.cellH) / layout.cellH };
  }

  // 過去コード列（現在スロットより左）。draw()と同じ並び: i=0が現在の直前(cursor-1)。
  // pastGeomsはサイズがスロットごとに違うため、矩形（縦横とも）で判定する。当たり判定は
  // PAST_HIT_MIN_SIZEを下限にして、極小スロットでもクリックできるようにする。
  if (x < layout.currentX) {
    for (const g of layout.pastGeoms) {
      const hitSize = Math.max(PAST_HIT_MIN_SIZE, g.size);
      if (px >= g.x - hitSize / 2 && px < g.x + hitSize / 2 && py >= g.y - hitSize / 2 && py < g.y + hitSize / 2) {
        return { kind: 'past', index: history.cursor - 1 - g.index, yRatio: 0.5 };
      }
    }
    return null;
  }

  // 未来コード列（現在スロットより右、候補ブロックより左）。固定サイズ・固定間隔のまま
  // （過去列と違い縮小しない）。i=0が記録済みの直後(cursor+1)。
  const futureCount = Math.min(MAX_FUTURE_SLOTS, history.entries.length - 1 - history.cursor);
  for (let i = 0; i < futureCount; i++) {
    const slotX = layout.currentX + FUTURE_SLOT_GAP * (i + 1);
    if (x >= slotX - FUTURE_SLOT_SIZE / 2 && x < slotX + FUTURE_SLOT_SIZE / 2) {
      return { kind: 'future', index: history.cursor + 1 + i, yRatio: 0.5 };
    }
  }
  return null;
}

function computeLayout(canvas) {
  const W = canvas.width;
  const H = canvas.height;
  // 過去/現在/未来スロットと候補ブロックは、ハンバーガーメニュー化で常時表示のUIが
  // canvas上から無くなったため、同じ縦領域（TOP_MARGIN〜H-BOTTOM_MARGIN）を共有する。
  const bodyH = Math.max(1, H - TOP_MARGIN - BOTTOM_MARGIN);
  // 候補セルは正方形（横長だとセル内上下の位置＝ベロシティの変化が実感しにくいため）。
  // 行数から決まる高さと、画面右端に収まる幅の両方で頭打ちにする（候補ブロックは右端寄せ）。
  const cellSize = Math.min(84, bodyH / assistRows, (W - RIGHT_MARGIN) / assistCols);
  const cellW = cellSize;
  const cellH = cellSize;
  const candidateX = W - assistCols * cellSize - RIGHT_MARGIN;
  // 現在スロットは、候補ブロックの左に未来スロット最大MAX_FUTURE_SLOTS個分の領域を
  // 確保した位置に置く。過去領域（0〜currentX）はウィンドウ幅に応じて自然に増減し、
  // 狭ければ過去スロットが入るだけ表示される（個数の固定上限は持たない）。
  const currentX = candidateX - (CURRENT_SIZE / 2 + MAX_FUTURE_SLOTS * FUTURE_SLOT_GAP + 16);
  // 候補ブロックは縦方向中央揃えで描く（draw()・cellFromPoint()の両方がここを基準にする）
  const candidateOriginY = TOP_MARGIN + (bodyH - assistRows * cellH) / 2;
  const slotY = TOP_MARGIN + bodyH / 2;
  const pastGeoms = computePastSlotGeoms({
    currentX,
    currentSize: CURRENT_SIZE,
    slotY,
    baseSize: PAST_BASE_SIZE,
    count: history.cursor,
    minSize: PAST_MIN_SIZE,
    shrink: PAST_SHRINK,
    gapRatio: PAST_GAP_RATIO,
    leftMargin: PAST_LEFT_MARGIN,
  });
  return { W, H, bodyH, currentX, candidateX, cellW, cellH, candidateOriginY, slotY, pastGeoms };
}

async function stopChord() {
  const notes = sounding;
  sounding = [];
  for (const note of notes) {
    await noteOff(CHORD_CHANNEL, note);
  }
}

/** ピボット経由の転調が確定したかを判定し、新しいkey/pendingPivotを返す（historyへは反映しない）。 */
function evaluateTheoryTransition(chord) {
  const key = currentKeyObj();
  const prevPivot = pendingPivotAt(history);
  let newKey = { tonicMidi, mode };
  if (prevPivot) {
    const confirmed = prevPivot.keys.find((k) => confirmsModulation(chord, k, key));
    if (confirmed) {
      newKey = { tonicMidi: 60 + confirmed.tonicPc, mode: confirmed.mode };
    }
  }
  const newKeyObj = { tonicPc: ((newKey.tonicMidi % 12) + 12) % 12, mode: newKey.mode };
  const pivots = pivotKeysFor(chord, newKeyObj);
  const newPendingPivot = pivots.length > 0 ? { keys: pivots } : null;
  return { key: newKey, pendingPivot: newPendingPivot };
}

function applyKey(newKey) {
  tonicMidi = newKey.tonicMidi;
  mode = newKey.mode;
  syncControlsFromState();
}

/**
 * 候補コードの選択を即座に確定する（発音は呼び出し側が行う）。選択は保持時間に関わらず確定する。
 * 編集操作なのでundoStackへ直前のhistoryを積み、redoStackは破棄する（一般的なUndo/Redoの規約）。
 * velocityは選択時に一度だけentryへ焼き付ける（過去/未来クリックでの再訪では変化しない）。
 */
function commitSelection(chord, velocity) {
  const { key, pendingPivot } = evaluateTheoryTransition(chord);
  undoStack.push(history);
  redoStack = [];
  history = selectChord(history, { chord, key, pendingPivot, velocity });
  applyKey(key);
  invalidateCandidates();
  startSlide(1);
}

/** 過去/未来の地点へ即座に再生位置を移動する（発音は呼び出し側が行う）。編集操作ではないためundo/redoスタックには積まない。 */
function jumpToIndex(index) {
  const direction = index < history.cursor ? -1 : 1;
  history = jumpTo(history, index);
  applyKey(keyAt(history));
  invalidateCandidates();
  startSlide(direction);
}

/** 直前のコード選択（過去へ戻った上での上書きも含む）を取り消し、その操作の直前のhistoryへ丸ごと復元する。 */
function undoEdit() {
  if (undoStack.length === 0) return;
  redoStack.push(history);
  history = undoStack.pop();
  applyKey(keyAt(history));
  invalidateCandidates();
  startSlide(-1);
}

/** undoEditで取り消したコード選択をやり直す。 */
function redoEdit() {
  if (redoStack.length === 0) return;
  undoStack.push(history);
  history = redoStack.pop();
  applyKey(keyAt(history));
  invalidateCandidates();
  startSlide(1);
}

async function playChord(chord, velocity) {
  await stopChord();
  await applyLfoTo(CHORD_CHANNEL);
  for (const note of chord.notes) {
    await noteOn(CHORD_CHANNEL, note, velocity);
  }
  sounding = chord.notes.slice();
}

export function setupChordScreen(canvas, { onChordChange } = {}) {
  canvas.addEventListener('mousemove', (e) => {
    if (!isActive('chord')) return;
    const cell = cellFromPoint(canvas, e.clientX, e.clientY);
    hoverCandidate = cell?.kind === 'candidate' ? cell : null;
    const nextHoverSlot = cell?.kind === 'past' || cell?.kind === 'future' ? cell : null;
    // 同じスロットに留まっている間はstartTimeを保持する（Dock風拡大アニメーションの起点。
    // 毎フレームリセットすると拡大が始まらない）。別スロットへ移ったときだけ計り直す。
    if (nextHoverSlot?.kind !== hoverSlot?.kind || nextHoverSlot?.index !== hoverSlot?.index) {
      hoverSlot = nextHoverSlot ? { ...nextHoverSlot, startTime: performance.now() } : null;
    } else if (nextHoverSlot) {
      hoverSlot = { ...hoverSlot, yRatio: nextHoverSlot.yRatio };
    }
  });

  canvas.addEventListener('mousedown', async (e) => {
    if (!isActive('chord') || e.button !== 0) return;
    const cell = cellFromPoint(canvas, e.clientX, e.clientY);
    if (!cell) return;
    pointerHeld = true;

    if (cell.kind === 'candidate') {
      const grid = computeCandidates();
      const found = grid.find((c) => c.col === cell.col && c.row === cell.row);
      if (!found) {
        pointerHeld = false;
        return;
      }
      // 選択は保持時間に関わらず確定する（離しても現在コードとして残る）
      const velocity = velocityFromCellY(cell.yRatio);
      commitSelection(found.chord, velocity);
      await playChord(found.chord, velocity);
    } else if (cell.kind === 'past' || cell.kind === 'future') {
      // 過去・未来どちらも「その地点へ再生位置を移動する」操作として対称に扱う
      jumpToIndex(cell.index);
      const entry = currentEntry(history);
      if (!entry) {
        pointerHeld = false;
        return;
      }
      await playChord(entry.chord, velocityFromCellY(cell.yRatio));
    }

    if (!pointerHeld) {
      await stopChord();
    }
    onChordChange?.(currentEntry(history)?.chord.name ?? null);
  });

  const release = async () => {
    pointerHeld = false;
    if (sounding.length === 0) return;
    await stopChord();
    onChordChange?.(currentEntry(history)?.chord.name ?? null);
  };
  canvas.addEventListener('mouseup', release);
  canvas.addEventListener('mouseleave', release);
  // 他の画面へ切り替えたときも、鳴りっぱなしを残さず止める（画面を跨いだ音の取りこぼし対策）
  onScreenChange((next) => {
    if (next !== 'chord') release();
  });

  window.addEventListener('keydown', (e) => {
    if (!isActive('chord')) return;
    if (e.key === 'Shift') {
      shiftHeld = true;
      invalidateCandidates();
    } else if (e.key === 'Control') {
      ctrlHeld = true;
      invalidateCandidates();
    } else if (e.key.toLowerCase() === 'z' && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      if (e.shiftKey) redoEdit();
      else undoEdit();
      onChordChange?.(currentEntry(history)?.chord.name ?? null);
    } else if (e.key.toLowerCase() === 'y' && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      redoEdit();
      onChordChange?.(currentEntry(history)?.chord.name ?? null);
    } else if (e.key === 'ArrowDown') {
      assistRows = Math.max(MIN_ROWS, assistRows - 1);
      invalidateCandidates();
      syncRowsCols();
    } else if (e.key === 'ArrowUp') {
      assistRows = Math.min(MAX_ROWS, assistRows + 1);
      invalidateCandidates();
      syncRowsCols();
    } else if (e.key === 'ArrowLeft') {
      assistCols = Math.max(MIN_COLS, assistCols - 1);
      invalidateCandidates();
      syncRowsCols();
    } else if (e.key === 'ArrowRight') {
      assistCols = Math.min(MAX_COLS, assistCols + 1);
      invalidateCandidates();
      syncRowsCols();
    }
  });
  window.addEventListener('keyup', (e) => {
    if (!isActive('chord')) return;
    if (e.key === 'Shift') {
      shiftHeld = false;
      invalidateCandidates();
    } else if (e.key === 'Control') {
      ctrlHeld = false;
      invalidateCandidates();
    }
  });
  // ウィンドウがフォーカスを失うとkeyupを取りこぼすため、押しっぱなし状態を解除する
  window.addEventListener('blur', () => {
    shiftHeld = false;
    ctrlHeld = false;
    invalidateCandidates();
  });

  return { draw: (ctx) => isActive('chord') && draw(ctx, canvas) };
}

let rowsInputEl = null;
let colsInputEl = null;
function syncRowsCols() {
  if (rowsInputEl) rowsInputEl.value = String(assistRows);
  if (colsInputEl) colsInputEl.value = String(assistCols);
}

/** 調・候補の行数/列数を切り替えるUIを配線する。 */
export function bindChordScreenControls({ tonicSelect, modeSelect, rowsInput, colsInput }) {
  tonicSelectEl = tonicSelect ?? null;
  modeSelectEl = modeSelect ?? null;
  rowsInputEl = rowsInput ?? null;
  colsInputEl = colsInput ?? null;

  if (tonicSelect) {
    NOTE_NAMES.forEach((name, i) => {
      const opt = document.createElement('option');
      opt.value = String(i);
      opt.textContent = name;
      tonicSelect.appendChild(opt);
    });
    tonicSelect.value = String(DEFAULT_TONIC_MIDI % 12);
    tonicSelect.addEventListener('change', async () => {
      // 調が変わると同じ候補が別のコードを指すため、鳴りっぱなしを避けて止める
      await stopChord();
      await allNotesOff(CHORD_CHANNEL);
      const pitchClass = parseInt(tonicSelect.value, 10) || 0;
      tonicMidi = 60 + pitchClass;
      resetHistory();
    });
  }

  if (modeSelect) {
    modeSelect.value = mode;
    modeSelect.addEventListener('change', async () => {
      await stopChord();
      await allNotesOff(CHORD_CHANNEL);
      mode = modeSelect.value === 'minor' ? 'minor' : 'major';
      resetHistory();
    });
  }

  if (rowsInput) {
    rowsInput.value = String(assistRows);
    rowsInput.addEventListener('input', () => {
      const raw = parseInt(rowsInput.value, 10) || DEFAULT_ROWS;
      assistRows = Math.max(MIN_ROWS, Math.min(MAX_ROWS, raw));
      invalidateCandidates();
    });
  }

  if (colsInput) {
    colsInput.value = String(assistCols);
    colsInput.addEventListener('input', () => {
      const raw = parseInt(colsInput.value, 10) || DEFAULT_COLS;
      assistCols = Math.max(MIN_COLS, Math.min(MAX_COLS, raw));
      invalidateCandidates();
    });
  }
}

/** 候補グリッドを、状態が変わったときだけ再計算してキャッシュする。 */
function computeCandidates() {
  const key = currentKeyObj();
  const entry = currentEntry(history);
  const cacheKey = JSON.stringify({
    from: entry ? { rootPc: entry.chord.rootPc, family: entry.chord.family } : null,
    key,
    shiftHeld,
    ctrlHeld,
    rows: assistRows,
    cols: assistCols,
  });
  if (candidateCache && candidateCache.cacheKey === cacheKey) return candidateCache.grid;

  const grid = computeCandidateGrid({
    lastChord: entry?.chord ?? null,
    key,
    tonicMidi,
    shiftHeld,
    ctrlHeld,
    cols: assistCols,
    rows: assistRows,
  });
  candidateCache = { cacheKey, grid };
  return grid;
}

/**
 * 選択/Undo/Redoの直後だけ発生する演出用の状態を返す。
 * offsetPx: 未来コード列だけの一時的な水平オフセット（0へ収束、従来どおり平行移動で演出）。
 *   選択（前進）なら+1スロット分右から、Undo（後退）なら-1スロット分左から現在位置へ戻る。
 * pastT: 過去コード列の位置・サイズ補間係数（0→1、draw()側でスロットごとにlerpする）。
 * candidateAlpha: 候補ブロックのフェードイン係数（0→1）。
 */
function currentSlideState() {
  if (slideDirection === 0) return { offsetPx: 0, pastT: 1, candidateAlpha: 1 };
  const elapsed = performance.now() - slideStart;
  const t = Math.min(1, elapsed / SLIDE_DURATION_MS);
  const eased = 1 - (1 - t) ** 2; // ease-out
  const startOffset = slideDirection > 0 ? FUTURE_SLOT_GAP : -FUTURE_SLOT_GAP;
  const offsetPx = startOffset * (1 - eased);
  if (t >= 1) slideDirection = 0;
  return { offsetPx, pastT: eased, candidateAlpha: eased };
}

/** Dock風ホバー拡大の補間係数（0→1、ease-out）。過去スロットのホバー中のみ意味を持つ。 */
function hoverExpandT() {
  if (!hoverSlot || hoverSlot.kind !== 'past') return 0;
  const elapsed = performance.now() - hoverSlot.startTime;
  const t = Math.min(1, elapsed / HOVER_EXPAND_MS);
  return 1 - (1 - t) ** 2;
}

function lerp(a, b, t) {
  return a + (b - a) * t;
}

/**
 * 選択時のベロシティを、スロット下端からの紺色の縦バーとして描く（0=バーなし、127=満タン）。
 * velocityがnull/undefinedなら何も描かない（entryが無い＝未選択の枠にバーを付けないため）。
 */
function drawVelocityBar(ctx, slotX, slotY, size, velocity, alpha) {
  if (velocity == null) return;
  const ratio = Math.max(0, Math.min(1, (velocity - VELOCITY_MIN) / (VELOCITY_MAX - VELOCITY_MIN)));
  if (ratio <= 0) return;
  const barH = size * ratio;
  ctx.fillStyle = `rgba(${VELOCITY_BAR_COLOR}, ${alpha})`;
  ctx.fillRect(slotX - size / 2, slotY + size / 2 - barH, size, barH);
}

/**
 * 過去/未来スロット1個分の描画（対称デザインなので共通化）。sizeが可変（過去列は遠いほど
 * 縮小する）ため、ラベルはサイズに応じてフル名／ルート音のみ／非表示を切り替える。
 */
function drawHistorySlot(ctx, chord, slotX, slotY, size, alpha, isHover, velocity) {
  ctx.fillStyle = isHover ? `rgba(150,190,255,${Math.min(1, alpha + 0.15)})` : `rgba(200,200,200,${alpha * 0.15})`;
  ctx.fillRect(slotX - size / 2, slotY - size / 2, size, size);
  drawVelocityBar(ctx, slotX, slotY, size, velocity, alpha * VELOCITY_BAR_ALPHA_SCALE);
  ctx.strokeStyle = `rgba(180,180,180,${alpha})`;
  ctx.strokeRect(slotX - size / 2 + 0.5, slotY - size / 2 + 0.5, size, size);

  const label = size >= PAST_LABEL_FULL_SIZE ? chord.name : size >= PAST_LABEL_ROOT_SIZE ? NOTE_NAMES[chord.rootPc] : null;
  if (label) {
    ctx.fillStyle = `rgba(220,220,220,${alpha})`;
    ctx.font = `${Math.max(9, Math.min(16, Math.floor(size / 5)))}px monospace`;
    ctx.textAlign = 'center';
    ctx.textBaseline = 'middle';
    ctx.fillText(label, slotX, slotY);
  }
}

function draw(ctx, canvas) {
  const layout = computeLayout(canvas);
  const { W, H, currentX, candidateX, cellW, cellH, candidateOriginY, slotY, pastGeoms } = layout;

  ctx.fillStyle = '#111';
  ctx.fillRect(0, 0, W, H);

  const { offsetPx, pastT, candidateAlpha } = currentSlideState();
  const entry = currentEntry(history);

  // 過去コード（現在スロットの左）。pastGeoms[i].indexが大きいほど遠い過去（cursor-1-index）。
  // 選択/Undo直後だけ、1つ現在寄りのスロット（i=0の開始点は現在スロットそのもの）から
  // pastTで位置・サイズを補間して飛んでくる。ホバー中のスロットはDock風拡大を後段で描くため、
  // ここでは描かずスキップする。
  for (let i = 0; i < pastGeoms.length; i++) {
    const g = pastGeoms[i];
    const idx = history.cursor - 1 - g.index;
    const past = history.entries[idx];
    if (!past) continue;
    if (hoverSlot?.kind === 'past' && hoverSlot.index === idx) continue;
    const startGeom =
      slideDirection > 0
        ? i === 0
          ? { x: currentX, y: slotY, size: CURRENT_SIZE }
          : pastGeoms[i - 1]
        : slideDirection < 0
          ? (pastGeoms[i + 1] ?? null)
          : null;
    const x = startGeom ? lerp(startGeom.x, g.x, pastT) : g.x;
    const y = startGeom ? lerp(startGeom.y, g.y, pastT) : g.y;
    const size = startGeom ? lerp(startGeom.size, g.size, pastT) : g.size;
    const targetAlpha = Math.max(PAST_ALPHA_MIN, PAST_ALPHA_BASE * PAST_ALPHA_DECAY ** g.index);
    const alpha = startGeom ? targetAlpha * pastT : targetAlpha;
    drawHistorySlot(ctx, past.chord, x, y, size, alpha, false, past.velocity);
  }

  // 未来コード（現在スロットの右）。固定サイズ・固定間隔のまま、平行移動のみで演出する
  ctx.save();
  if (offsetPx !== 0) ctx.translate(offsetPx, 0);
  const futureCount = Math.min(MAX_FUTURE_SLOTS, history.entries.length - 1 - history.cursor);
  for (let i = 0; i < futureCount; i++) {
    const idx = history.cursor + 1 + i;
    const future = history.entries[idx];
    if (!future) continue;
    const slotX = currentX + FUTURE_SLOT_GAP * (i + 1);
    const alpha = 0.75 - i * 0.18;
    const isHover = hoverSlot?.kind === 'future' && hoverSlot.index === idx;
    drawHistorySlot(ctx, future.chord, slotX, slotY, FUTURE_SLOT_SIZE, alpha, isHover, future.velocity);
  }
  ctx.restore();

  // 現在コードのスロット（固定位置）
  {
    const size = CURRENT_SIZE;
    const x = currentX;
    const y = slotY;
    ctx.fillStyle = sounding.length > 0 ? 'rgba(120,200,255,0.12)' : 'rgba(255,255,255,0.04)';
    ctx.fillRect(x - size / 2, y - size / 2, size, size);
    drawVelocityBar(ctx, x, y, size, entry?.velocity, VELOCITY_BAR_ALPHA_SCALE);
    ctx.strokeStyle = '#eee';
    ctx.lineWidth = 2;
    ctx.strokeRect(x - size / 2 + 1, y - size / 2 + 1, size - 2, size - 2);
    ctx.fillStyle = '#fff';
    ctx.font = 'bold 22px monospace';
    ctx.textAlign = 'center';
    ctx.textBaseline = 'middle';
    ctx.fillText(entry ? entry.chord.name : '—', x, y);
    ctx.lineWidth = 1;
  }

  // ホバー中の過去スロットをDock風に拡大して手前へ再描画する（半透明オーバーレイと同じ理由で、
  // 通常描画より後に描く必要がある。[[project_gesture_app_3screen_minidaw_redesign]]で踏んだ
  // 「半透明オーバーレイは対象要素より後に描く」教訓の応用）。
  if (hoverSlot?.kind === 'past') {
    const hoverGeom = pastGeoms.find((g) => history.cursor - 1 - g.index === hoverSlot.index);
    const hoverEntry = history.entries[hoverSlot.index];
    if (hoverGeom && hoverEntry) {
      const size = lerp(hoverGeom.size, HOVER_EXPAND_SIZE, hoverExpandT());
      drawHistorySlot(ctx, hoverEntry.chord, hoverGeom.x, hoverGeom.y, size, 1, true, hoverEntry.velocity);
    }
  }

  // 候補ブロック（現在スロットの右）。過去コード＋現在スロットの移動には追従させず、
  // 新しい状態へのフェードインだけを演出する。
  ctx.save();
  ctx.globalAlpha = candidateAlpha;
  const grid = computeCandidates();
  for (const cell of grid) {
    const x = candidateX + cell.col * cellW;
    const y = candidateOriginY + cell.row * cellH;
    const clampedScore = Math.max(0, Math.min(1, cell.score / MAX_SCORE_FOR_SHADING));
    const alpha = 0.15 + clampedScore * 0.45;
    ctx.fillStyle =
      cell.category === 'GREEN'
        ? `hsla(120, 100%, 50%, ${alpha})` // 00FF00（最も明るい緑）と同じhue/sat/lightness
        : cell.category === 'YELLOW'
          ? `hsla(48, 75%, 50%, ${alpha})`
          : `hsla(0, 0%, 50%, ${alpha * 0.6})`;
    ctx.fillRect(x + 1, y + 1, cellW - 2, cellH - 2);
    if (cell.isPivot) {
      ctx.fillStyle = `hsla(210, 90%, 60%, ${0.18 + clampedScore * 0.12})`;
      ctx.fillRect(x + 1, y + 1, cellW - 2, cellH - 2);
    }
    ctx.strokeStyle = '#3a3a3a';
    ctx.strokeRect(Math.round(x) + 0.5, Math.round(y) + 0.5, cellW, cellH);
  }

  // ホバー中の候補セル。塗りの濃さがそのままベロシティの目安になる（候補セル本体より後に描く）
  if (hoverCandidate) {
    const velocity = velocityFromCellY(hoverCandidate.yRatio);
    const alpha = 0.08 + (velocity / 127) * 0.18;
    const x = candidateX + hoverCandidate.col * cellW;
    const y = candidateOriginY + hoverCandidate.row * cellH;
    ctx.fillStyle = `rgba(150, 190, 255, ${alpha})`;
    ctx.fillRect(x + 1, y + 1, cellW - 2, cellH - 2);
  }

  // 候補セル名
  ctx.font = `${Math.max(9, Math.min(14, Math.floor(cellW / 6)))}px monospace`;
  ctx.fillStyle = '#ddd';
  ctx.textAlign = 'center';
  ctx.textBaseline = 'middle';
  for (const cell of grid) {
    const x = candidateX + cell.col * cellW + cellW / 2;
    const y = candidateOriginY + cell.row * cellH + cellH / 2;
    ctx.fillText(cell.chord.name, x, y);
  }
  ctx.textBaseline = 'alphabetic';
  ctx.restore(); // globalAlphaを戻す

  drawLayerHint(ctx, W, H);
}

function drawLayerHint(ctx, W, H) {
  const label =
    ctrlHeld && shiftHeld
      ? 'aug/オルタードレイヤー (Ctrl+Shift)'
      : ctrlHeld
        ? 'sus/付加音レイヤー (Ctrl)'
        : shiftHeld
          ? '4和音レイヤー (Shift)'
          : 'トライアドレイヤー';
  const color = ctrlHeld || shiftHeld ? '#4af' : '#555';
  ctx.textAlign = 'right';
  ctx.font = '13px monospace';
  ctx.fillStyle = color;
  const keyLabel = `${NOTE_NAMES[((tonicMidi % 12) + 12) % 12]} ${mode === 'minor' ? 'Minor' : 'Major'}`;
  ctx.fillText(`${label}　調 = ${keyLabel}`, W - 18, H - 22);
}
