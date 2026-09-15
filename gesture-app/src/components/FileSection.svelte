<script lang="ts">
  // Fileメニュー（Open/Save/Save As、独自プロジェクト形式.gap505）＋MIDI Import/Export（.mid）。
  // 戻り値のboolean（旧: refreshTempoDisplay()を呼ぶかの判定に使っていた）はテンポ表示が
  // $state化された今は不要なので無視する。
  import { openProject, saveProject, saveProjectAs, importMidi, exportMidi } from '../project-file.ts';
</script>

<div id="file-section">
  <div class="drawer-section-title">FILE</div>
  <div class="program-row">
    <button type="button" id="file-open-btn" onclick={() => openProject()}>Open</button>
    <button type="button" id="file-save-btn" onclick={() => saveProject()}>Save</button>
    <button type="button" id="file-save-as-btn" onclick={() => saveProjectAs()}>Save As</button>
  </div>
  <div class="program-row">
    <button type="button" id="file-import-btn" onclick={() => importMidi()}>Import .mid</button>
    <button type="button" id="file-export-btn" onclick={() => exportMidi()}>Export .mid</button>
  </div>
</div>
