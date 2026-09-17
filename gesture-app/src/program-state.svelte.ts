// 音源モード切替（波形メモリ ⇔ Bank/Program手動指定）とProgram切り替え（動作確認用の簡易UI）。
// デフォルトはFM音源（チェックOFF）。チェックを切り替えるたびに切り替え前のBank/Programを
// そのモード用に退避し、切り替え後のモードで前回使っていたBank/Programを復元する（FM⇔波形
// メモリのどちら向きの切替でも双方向に復元される）。「波形メモリ」チェックON時はBank欄を
// WAVEFORM_MEMORY_BANKに固定して編集不可にする。段階Cで main.ts のIIFEから切り出し、$state化。

import { getCurrentWindow } from '@tauri-apps/api/window';
import { isTauri } from '@tauri-apps/api/core';
import { setProgram, queryProgramName, CHORD_CHANNEL, MELODY_CHANNEL, RHYTHM_CHANNEL } from './midi.ts';
import { activeScreen, onScreenChange } from './screens.svelte.ts';
import type { ProgramInfo } from './types.ts';

/** 今アクティブな画面が音色を送受信するMIDIチャンネル。CHORD/MELODY/RHYTHM各画面は
 * それぞれ別チャンネルの音色を持つため、Bank/Program欄の送信先・表示元もこれで決まる
 * （旧実装は画面によらず常にCHORD_CHANNEL固定だったため、MELODY/RHYTHM画面で音色選択
 * しても反映されない不具合があった）。Eキーでの音色エディタ起動（`main.ts`）も同じ
 * チャンネルをEdit Channelの初期値としてstandaloneへ渡すため、exportする。 */
export function activeProgramChannel(): number {
  switch (activeScreen()) {
    case 'melody':
      return MELODY_CHANNEL;
    case 'rhythm':
      return RHYTHM_CHANNEL;
    default:
      return CHORD_CHANNEL;
  }
}

// 波形メモリ音色専用のBank Select番号（凍結済みym38x6-coreのWAVEFORM_MEMORY_BANKと一致させていた
// 値）。op505向けの音色は2026-08-25に移植済み: op505-coreにはこのBankを特別扱いするフォール
// バックコードは無く、代わりに`op505/tools/patchlab/python/waveform_memory_bank.py`が生成した
// 実体の.op505ファイルを通常のプリセットバンクとして配置してある。音色名自体はstandaloneへの
// 問い合わせ（queryProgramName）で得るため、ここではBank欄の固定にのみ使う。
const WAVEFORM_MEMORY_BANK = 16383;

export const programState: { waveformMemory: boolean; bank: number; program: number; label: string } = $state({
  waveformMemory: false,
  bank: 0,
  program: 0,
  label: '',
});

// 各モードで最後に使っていたBank/Program（モード切替時の復元先）
let savedFmBank = 0;
let savedFmProgram = 0;
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

export async function refreshProgramLabel(): Promise<void> {
  const info = await queryProgramName(activeProgramChannel());
  programState.label = formatProgramInfo(info);
}

/** 画面切替時・ウィンドウフォーカス復帰時に呼ぶ。その画面のチャンネルの実際の音色を
 * standaloneへ問い合わせてBank/Program欄へ反映するだけで、何も送信はしない
 * （CHORD/MELODY/RHYTHM各画面は別チャンネルの音色を持つため、表示側もアクティブ画面に
 * 合わせて切り替える必要がある。フォーカス復帰時に呼ぶのは、音色エディタでBank/Programを
 * 変更してgesture-appへ戻ってきた場合に、表示だけでなく実際のBank/Program欄の値も
 * 追随させるため——`refreshProgramLabel`は表示ラベルしか更新しないため不十分だった）。 */
export async function syncFromActiveChannel(): Promise<void> {
  const info = await queryProgramName(activeProgramChannel());
  programState.waveformMemory = info.bank === WAVEFORM_MEMORY_BANK;
  programState.bank = info.bank;
  programState.program = info.program;
  programState.label = formatProgramInfo(info);
}

onScreenChange(() => {
  syncFromActiveChannel();
});

function syncBankField(): void {
  if (programState.waveformMemory) {
    // FM → 波形メモリ：現在のFM Bank/Programを退避し、波形メモリ側の前回Programを復元
    savedFmBank = programState.bank;
    savedFmProgram = programState.program;
    programState.bank = WAVEFORM_MEMORY_BANK;
    programState.program = savedWmProgram;
  } else {
    // 波形メモリ → FM：現在のProgramを退避し、FM側のBank/Programを復元
    savedWmProgram = programState.program;
    programState.bank = savedFmBank;
    programState.program = savedFmProgram;
  }
}

async function applyProgram(): Promise<void> {
  const bank = programState.waveformMemory ? WAVEFORM_MEMORY_BANK : Math.max(0, Math.min(16383, programState.bank));
  const program = Math.max(0, Math.min(127, programState.program));
  // Bank Select + Program Changeを送るだけ（見つかるかどうかの判断はstandalone任せ）。
  await setProgram(activeProgramChannel(), bank, program);
  await refreshProgramLabel();
}

export async function setWaveformMemory(on: boolean): Promise<void> {
  programState.waveformMemory = on;
  syncBankField();
  await applyProgram();
}

export async function setBank(bank: number): Promise<void> {
  programState.bank = Math.max(0, Math.min(16383, Number.isFinite(bank) ? bank : 0));
  await applyProgram();
}

export async function setProgramNumber(program: number): Promise<void> {
  programState.program = Math.max(0, Math.min(127, Number.isFinite(program) ? program : 0));
  await applyProgram();
}

/** 起動時の初期化。既定の音色（OP505 Bank0/Program0）を反映し、ウィンドウフォーカス復帰時の
 * 再問い合わせも配線する（Eキーでstandaloneのトレイ起動音色エディタを開いて閉じた場合、
 * standaloneのタスクトレイメニューから開いて閉じた場合、Domino等で別音色を鳴らしてから
 * 戻ってきた場合も、このイベント1つでBank/Program欄・表示ラベルとも追随する。エディタは
 * 別ウィンドウのため、閉じれば必ずgesture-appへフォーカスが戻る）。 */
export function initProgramState(): void {
  syncBankField();
  applyProgram();
  if (isTauri()) {
    getCurrentWindow().onFocusChanged(({ payload: focused }) => {
      if (focused) syncFromActiveChannel();
    });
  }
}
