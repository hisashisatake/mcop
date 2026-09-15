// エントリポイント。画面（現状はコード画面のみ）の起動と、画面共通のパネル類を配線する。

import { getCurrentWindow } from '@tauri-apps/api/window';
import { isTauri } from '@tauri-apps/api/core';
import { setProgram, tapTempo, openEditor, queryProgramName, setSequencerRunning, CHORD_CHANNEL } from './midi.ts';
import { setupMidiLog } from './midi-log.ts';
import { setupPerformanceLfo, bindLfoIndicator } from './performance-lfo.ts';
import { setupChordScreen, bindChordScreenControls, activeChannels } from './chord-screen.ts';
import { setupRhythmScreen, bindRhythmScreenControls, resetRhythmCursor } from './rhythm-screen.ts';
import { setupMelodyScreen, resetMelodyCursor, deleteSelectedMelodyNote } from './melody-screen.ts';
import { activeScreen, bindScreenTabs, onScreenChange } from './screens.ts';
import { getBpm, setBpm } from './tempo-state.ts';
import { undo, redo } from './undo-manager.ts';
import { openProject, saveProject, saveProjectAs, importMidi, exportMidi } from './project-file.ts';
import type { ProgramInfo } from './types.ts';

function byId<T extends HTMLElement>(id: string): T {
  return document.getElementById(id) as T;
}

setupMidiLog(byId('midi-log'));

// 波形メモリ音色専用のBank Select番号（凍結済みym38x6-coreのWAVEFORM_MEMORY_BANKと一致させていた
// 値）。op505向けの音色は2026-08-25に移植済み: op505-coreにはこのBankを特別扱いする
// フォールバックコードは無く（op505は実行時コード生成パターン自体を廃止済み）、代わりに
// `op505/tools/patchlab/python/waveform_memory_bank.py`が生成した実体の.op505ファイルを
// 通常のプリセットバンクとして%USERPROFILE%\Documents\op505\presets\へ配置してある。
// 音色名自体はstandaloneへの問い合わせ（queryProgramName）で得るため、ここでは
// Bank欄の固定にのみ使う。
const WAVEFORM_MEMORY_BANK = 16383;

// ─────────────────────────────────────────────
// Canvas
// ─────────────────────────────────────────────
const canvas = byId<HTMLCanvasElement>('canvas');
const ctx = canvas.getContext('2d')!;
const chordEl = byId('chord-display');
const chordFunctionEl = byId('chord-function');
const chordNoteNameEl = byId('chord-note-name');

function resize(): void {
  canvas.width = window.innerWidth;
  canvas.height = window.innerHeight;
}
window.addEventListener('resize', resize);
resize();

// 右下の手動リサイズグリップ。Tauri/WebView2の既知の不具合（decorations有りウィンドウで
// 右端・下端(East/South)のドラッグがOSのNCHITTESTを素通りしリサイズできない。
// 左端・上端(West/North)は正常、tauri-apps/tauri#9023）の回避策。
// startResizeDragging()はOSのネイティブ拡縮ループを直接起動するため、
// 壊れているNCHITTEST判定を経由せずリサイズが機能する。
// 'East'/'South'単体は実機検証で無反応だったため使わず、動作を確認できた
// 'SouthEast'（斜め）のみ使う——横だけ/縦だけ動かせば実質その方向だけのリサイズになる。
byId('resize-grip').addEventListener('mousedown', async (e) => {
  e.preventDefault();
  if (!isTauri()) return;
  await getCurrentWindow().startResizeDragging('SouthEast');
});

// ─────────────────────────────────────────────
// 音源モード切替（波形メモリ ⇔ Bank/Program手動指定）とProgram切り替え
// （動作確認用の簡易UI）。デフォルトはFM音源（チェックOFF）。チェックを切り替えるたびに
// 切り替え前のBank/Programをそのモード用に退避し、切り替え後のモードで前回使っていた
// Bank/Programを復元する（FM⇔波形メモリのどちら向きの切替でも双方向に復元される）。
// 「波形メモリ」チェックON時はBank欄をWAVEFORM_MEMORY_BANKに固定して編集不可にする。
// ─────────────────────────────────────────────
(() => {
  const wmToggle = byId<HTMLInputElement>('waveform-memory-toggle');
  const bankEl = byId<HTMLInputElement>('program-bank');
  const numEl = byId<HTMLInputElement>('program-num');
  const labelEl = byId('program-label');

  // 各モードで最後に使っていたBank/Program（モード切替時の復元先）
  let savedFmBank = parseInt(bankEl.value, 10) || 0;
  let savedFmProgram = parseInt(numEl.value, 10) || 0;
  let savedWmProgram = 0;

  // standaloneへの問い合わせ結果（Rust側`ProgramInfoDto`のstatus）を表示文字列へ変換する。
  // 音色名の正解はstandaloneが持つ`.op505`プリセットのみであり、ここでは名前を推測しない
  // （memory `project_gesture_app_program_name_standalone_query.md`参照）。
  function formatProgramInfo(info: ProgramInfo): string {
    switch (info.status) {
      case 'disconnected':
        return 'standalone未接続';
      case 'resolved':
        return info.name ?? '';
      case 'not_found':
        return `Bank ${info.bank} / Program ${info.program}（.op505未登録）`;
      case 'rhythm':
        return `Rhythm Kit ${info.program}`;
      case 'editing':
        return '音色エディタ編集中';
      default:
        return `Bank ${info.bank} / Program ${info.program}`;
    }
  }

  async function refreshProgramLabel(): Promise<void> {
    const info = await queryProgramName(CHORD_CHANNEL);
    labelEl.textContent = formatProgramInfo(info);
  }

  function syncBankField(): void {
    if (wmToggle.checked) {
      // FM → 波形メモリ：現在のFM Bank/Programを退避し、波形メモリ側の前回Programを復元
      savedFmBank = parseInt(bankEl.value, 10) || 0;
      savedFmProgram = parseInt(numEl.value, 10) || 0;
      bankEl.value = String(WAVEFORM_MEMORY_BANK);
      numEl.value = String(savedWmProgram);
    } else {
      // 波形メモリ → FM：現在のProgramを退避し、FM側のBank/Programを復元
      savedWmProgram = parseInt(numEl.value, 10) || 0;
      bankEl.value = String(savedFmBank);
      numEl.value = String(savedFmProgram);
    }
    bankEl.disabled = wmToggle.checked;
  }

  async function applyProgram(): Promise<void> {
    const bank = wmToggle.checked
      ? WAVEFORM_MEMORY_BANK
      : Math.max(0, Math.min(16383, parseInt(bankEl.value, 10) || 0));
    const program = Math.max(0, Math.min(127, parseInt(numEl.value, 10) || 0));

    // Bank Select + Program Changeを送るだけ（見つかるかどうかの判断はstandalone任せ）。
    await setProgram(bank, program);
    await refreshProgramLabel();
  }

  wmToggle.addEventListener('change', () => {
    syncBankField();
    applyProgram();
  });
  bankEl.addEventListener('input', applyProgram);
  numEl.addEventListener('input', applyProgram);

  syncBankField();
  applyProgram(); // 起動時に既定の音色（OP505 Bank0/Program0）を反映

  // ウィンドウフォーカス復帰時に再問い合わせる。Eキー押下でstandaloneのトレイ起動音色
  // エディタを開いて閉じた場合も、standaloneのタスクトレイメニューから開いて閉じた場合も、
  // Domino等で別音色を鳴らしてから戻ってきた場合も、このイベント1つで表示が追随する
  // （エディタは別ウィンドウのため、閉じれば必ずgesture-appへフォーカスが戻る）。
  if (isTauri()) {
    getCurrentWindow().onFocusChanged(({ payload: focused }) => {
      if (focused) refreshProgramLabel();
    });
  }
})();

// ─────────────────────────────────────────────
// タップテンポ（MC-505のタップボタン相当）
// TimeEgのテンポ同期（sync_enabled）が対象にするBPMを、ボタン連打の間隔から算出する。
// 3回タップ（間隔2区間）でBPM確定、以降のタップは直近区間の移動平均で更新し続ける。
// タップ間隔が2秒を超えたら系列をリセットする（別のテンポで叩き直したいときの意図を汲む）。
// 確定したBPMはtap_tempoコマンド経由でstandaloneへ送られ、MIDI Clock(0xF8)として
// 実際に送出される（クロック送信スレッド自体はsrc-tauri/src/midi_out.rsが持つ）。
// ─────────────────────────────────────────────
const tempoDisplayEl = byId('tempo-display');

/** tempo-state.jsの現在値を#tempo-displayへ反映する。Undo/Redo・ファイル読込でBPMが
 * 変わった際、main.js側からこれを呼んで表示を追随させる（tapTempo()自体はMIDI Clock送出
 * のみでDOM更新は行わないため）。 */
function refreshTempoDisplay(): void {
  const bpm = getBpm();
  tempoDisplayEl.textContent = bpm == null ? '— BPM' : `${Math.round(bpm)} BPM`;
}
refreshTempoDisplay();

(() => {
  const tapBtn = byId('tap-tempo-btn');
  const MIN_BPM = 40;
  const MAX_BPM = 300;
  const RESET_GAP_MS = 2000;
  const MOVING_AVERAGE_WINDOW = 4; // 直近何区間を平均するか

  let tapTimestamps: number[] = [];

  tapBtn.addEventListener('click', async () => {
    const now = performance.now();
    if (tapTimestamps.length > 0 && now - tapTimestamps[tapTimestamps.length - 1] > RESET_GAP_MS) {
      tapTimestamps = [];
    }
    tapTimestamps.push(now);
    if (tapTimestamps.length > MOVING_AVERAGE_WINDOW + 1) {
      tapTimestamps.shift();
    }
    if (tapTimestamps.length < 3) {
      return; // 3タップ（2区間）が揃うまではBPMを確定しない
    }
    const intervals: number[] = [];
    for (let i = 1; i < tapTimestamps.length; i++) {
      intervals.push(tapTimestamps[i] - tapTimestamps[i - 1]);
    }
    const avgIntervalMs = intervals.reduce((a, b) => a + b, 0) / intervals.length;
    const bpm = Math.max(MIN_BPM, Math.min(MAX_BPM, 60000 / avgIntervalMs));
    setBpm(bpm);
    refreshTempoDisplay();
    await tapTempo(bpm);
  });
})();

// ─────────────────────────────────────────────
// 画面切り替え（コード/リズム/メロディの3画面。フェーズ3）
// ─────────────────────────────────────────────
const chordScreen = setupChordScreen(canvas, {
  onChordChange: (info) => {
    chordEl.textContent = info?.degreeLabel ?? '—';
    chordFunctionEl.textContent = info?.func ?? '';
    chordNoteNameEl.textContent = info?.noteName ?? '';
  },
});

bindChordScreenControls({
  tonicSelect: byId<HTMLSelectElement>('key-tonic'),
  modeSelect: byId<HTMLSelectElement>('key-mode'),
  rowsInput: byId<HTMLInputElement>('candidate-rows'),
  colsInput: byId<HTMLInputElement>('candidate-cols'),
  autoVoicingToggle: byId<HTMLInputElement>('auto-voicing-toggle'),
  baseOctaveInput: byId<HTMLInputElement>('base-octave'),
});

const rhythmScreen = setupRhythmScreen(canvas);
bindRhythmScreenControls({
  metronomeToggle: byId<HTMLInputElement>('metronome-toggle'),
});

const melodyScreen = setupMelodyScreen(canvas);

// リズム/メロディ共通の再生/停止ボタン（メニューバー）。両画面のパターンを同時に
// 再生/停止する。停止時は両画面の再生カーソルも揃えてリセットする（Rust側は
// `rhythm-step`/`melody-step`イベントの送出を止めるだけで、直前のカーソル位置は
// 明示的に片付けてくれないため）。
const sequencerPlayBtn = byId<HTMLButtonElement>('sequencer-play-btn');
const sequencerStopBtn = byId<HTMLButtonElement>('sequencer-stop-btn');
function updateSequencerButtons(running: boolean): void {
  sequencerPlayBtn.disabled = running;
  sequencerStopBtn.disabled = !running;
}
updateSequencerButtons(false);
sequencerPlayBtn.addEventListener('click', () => {
  updateSequencerButtons(true);
  setSequencerRunning(true);
});
sequencerStopBtn.addEventListener('click', () => {
  updateSequencerButtons(false);
  resetRhythmCursor();
  resetMelodyCursor();
  setSequencerRunning(false);
});

bindScreenTabs({
  chord: byId('tab-chord'),
  rhythm: byId('tab-rhythm'),
  melody: byId('tab-melody'),
});

// ハンバーガーメニュー: 画面タブ・ヒント・MIDIログ・音源/演奏設定を収めたドロワーの開閉。
// 画面を切り替えたら、選んだ画面がすぐ見えるようドロワーを自動で閉じる。
const menuToggleEl = byId('menu-toggle');
const drawerEl = byId('drawer');
menuToggleEl.addEventListener('click', () => drawerEl.classList.toggle('open'));
onScreenChange(() => drawerEl.classList.remove('open'));

// 画面ごとのコントロールパネル・キーボードヒントの出し分け
const chordControlsEl = byId('chord-controls');
const rhythmControlsEl = byId('rhythm-controls');
const statusKeyRowEl = byId('status-key-row'); // 常時表示の#status-panel内、Key選択はコード画面専用
const hintEl = byId('hint');
const CHORD_HINT = hintEl.innerHTML;
const RHYTHM_HINT = '<div class="drawer-section-title">操作</div>クリック: セルのベロシティを巡回（消音→通常→アクセント→弱）<br>再生/停止は左上メニューバーのアイコンから（再生にはTAPでテンポ確定が必要）<br>メトロノームON/OFFは下の音源パネルから<br>E: 音色エディタ';
const MELODY_HINT = '<div class="drawer-section-title">操作</div>空セルをクリック+ドラッグ: ノート作成（長さ調整）<br>ノートをクリック: 選択<br>選択中ノートをクリック: 音量を巡回（通常→アクセント→弱）<br>選択中ノートをドラッグ: 移動<br>ノート右端をドラッグ: 長さ変更<br>DEL: 選択中ノートを削除<br>ホイール: 音域スクロール　Shift+ホイール: 小節スクロール<br>再生/停止はリズムと共通（左上メニューバー）<br>E: 音色エディタ';

onScreenChange((next) => {
  chordControlsEl.hidden = next !== 'chord';
  rhythmControlsEl.hidden = next !== 'rhythm';
  statusKeyRowEl.hidden = next !== 'chord';
  sequencerPlayBtn.hidden = next !== 'rhythm' && next !== 'melody';
  sequencerStopBtn.hidden = next !== 'rhythm' && next !== 'melody';
  hintEl.innerHTML = next === 'chord' ? CHORD_HINT : next === 'rhythm' ? RHYTHM_HINT : MELODY_HINT;
});

setupPerformanceLfo(canvas, activeChannels);
bindLfoIndicator({
  label: byId('lfo-label'),
  depthBar: byId('lfo-depth-bar'),
  rateLabel: byId('lfo-rate-label'),
  rateBar: byId('lfo-rate-bar'),
});

// Ctrl+Z/Ctrl+Yは統合Undo/Redo（undo-manager.js）として、アクティブな画面を問わず
// グローバルに配線する（CHORD画面だけがアクティブな時に効く、という以前の制約を撤廃）。
// undo()/redo()はプロジェクト全体を復元するだけでDOM更新は行わないため、BPM表示だけは
// ここで明示的にrefreshTempoDisplay()する（コード/リズム/メロディの画面自体は各screen
// モジュール内の状態更新とアニメーションループの再描画で自動的に追随する）。
window.addEventListener('keydown', async (e) => {
  if (e.key.toLowerCase() === 'e') {
    await openEditor();
  } else if ((e.key === 'Delete' || e.key === 'Backspace') && activeScreen() === 'melody') {
    deleteSelectedMelodyNote();
  } else if (e.key.toLowerCase() === 'z' && (e.ctrlKey || e.metaKey)) {
    e.preventDefault();
    if (e.shiftKey) redo();
    else undo();
    refreshTempoDisplay();
  } else if (e.key.toLowerCase() === 'y' && (e.ctrlKey || e.metaKey)) {
    e.preventDefault();
    redo();
    refreshTempoDisplay();
  }
});

// ─────────────────────────────────────────────
// Fileメニュー（Open/Save/Save As、独自プロジェクト形式.gap505）
// ─────────────────────────────────────────────
byId('file-open-btn').addEventListener('click', async () => {
  if (await openProject()) refreshTempoDisplay();
});
byId('file-save-btn').addEventListener('click', () => saveProject());
byId('file-save-as-btn').addEventListener('click', () => saveProjectAs());

// MIDI Import/Export（フェーズ3、リズム/メロディの2ch分のみ対象、コードは対象外）
byId('file-import-btn').addEventListener('click', async () => {
  if (await importMidi()) refreshTempoDisplay();
});
byId('file-export-btn').addEventListener('click', () => exportMidi());

// ─────────────────────────────────────────────
// アニメーションループ
// ─────────────────────────────────────────────
function tick(): void {
  const screen = activeScreen();
  if (screen === 'rhythm') rhythmScreen.draw(ctx);
  else if (screen === 'melody') melodyScreen.draw(ctx);
  else chordScreen.draw(ctx);
  requestAnimationFrame(tick);
}

tick();
