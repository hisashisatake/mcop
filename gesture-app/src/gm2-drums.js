// GM2パーカッションキーマップ（ノート番号→名前）。フェーズ3のMIDI Import/Exportで、
// 既定12行に無いノート番号（Pedal HH=44、Crash 2=57等）が出てきた際のラベル表示に使う。
// 既定12行自体のラベル（Crash/Ride/OpenHH等）はrhythm-screen.jsが個別に持つ短縮名を
// 優先するため、ここには含めない（rowLabel()参照）。

const GM2_PERCUSSION_NAMES = {
  27: 'High Q', 28: 'Slap', 29: 'Scratch Push', 30: 'Scratch Pull', 31: 'Sticks',
  32: 'Square Click', 33: 'Metronome Click', 34: 'Metronome Bell',
  35: 'Ac. Bass Drum', 36: 'Bass Drum 1', 37: 'Side Stick', 38: 'Ac. Snare',
  39: 'Hand Clap', 40: 'El. Snare', 41: 'Low Floor Tom', 42: 'Closed HH',
  43: 'High Floor Tom', 44: 'Pedal HH', 45: 'Low Tom', 46: 'Open HH',
  47: 'Low-Mid Tom', 48: 'Hi-Mid Tom', 49: 'Crash 1', 50: 'High Tom',
  51: 'Ride 1', 52: 'Chinese Cymbal', 53: 'Ride Bell', 54: 'Tambourine',
  55: 'Splash Cymbal', 56: 'Cowbell', 57: 'Crash 2', 58: 'Vibraslap',
  59: 'Ride 2', 60: 'Hi Bongo', 61: 'Low Bongo', 62: 'Mute Hi Conga',
  63: 'Open Hi Conga', 64: 'Low Conga', 65: 'High Timbale', 66: 'Low Timbale',
  67: 'High Agogo', 68: 'Low Agogo', 69: 'Cabasa', 70: 'Maracas',
  71: 'Short Whistle', 72: 'Long Whistle', 73: 'Short Guiro', 74: 'Long Guiro',
  75: 'Claves', 76: 'Hi Wood Block', 77: 'Low Wood Block', 78: 'Mute Cuica',
  79: 'Open Cuica', 80: 'Mute Triangle', 81: 'Open Triangle', 82: 'Shaker',
  83: 'Jingle Bell', 84: 'Belltree', 85: 'Castanets', 86: 'Mute Surdo', 87: 'Open Surdo',
};

/** GM2ノート番号(0〜127)から表示名を返す。テーブル外は`Note ##`にフォールバックする。 */
export function gm2DrumName(note) {
  return GM2_PERCUSSION_NAMES[note] ?? `Note ${note}`;
}
