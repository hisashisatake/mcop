//! gesture-appの音色名クエリクライアント。standalone側の`op505/standalone/src/query_server.rs`
//! と対になる、クエリ専用パイプ`\\.\pipe\op505.query.v1`への単発リクエスト/レスポンス実装。
//!
//! `midi_out.rs`と違い常駐ライタースレッド・再接続ループは持たない。呼ばれるたびに
//! 接続→書く→読む→閉じるだけで完結する（1リクエスト1レスポンスをその場で返す
//! サーバー設計のため、常駐接続を維持する動機が無い）。standalone未起動時は
//! `OpenOptions::open`が即座にエラーを返すので、それをそのまま「未接続」として扱う。
//! 明示的なタイムアウト機構は設けない（サーバー側がブロックする理由が無いため）。
//!
//! フレーム形式（query_server.rsのdoc・decode_query_request/encode_program_info_responseと
//! 対になる。変更時は両方揃える）:
//! `[u8 version=1][u8 kind][u8 reserved][u16 len（リトルエンディアン）][bytes; len]`
//! - リクエスト kind=0（QueryProgram）: payload = `[channel]`（0〜15）
//! - レスポンス kind=0x80（ProgramInfo）: payload =
//!   `[status][bank_lo][bank_hi][program][name_len][name UTF-8 bytes; name_len]`

use std::fs::OpenOptions;
use std::io::{Read, Write};

const PIPE_PATH: &str = r"\\.\pipe\op505.query.v1";
const FRAME_VERSION: u8 = 1;
const REQUEST_KIND_QUERY_PROGRAM: u8 = 0;
const RESPONSE_KIND_PROGRAM_INFO: u8 = 0x80;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProgramStatus {
    /// `.op505`プリセットから名前が見つかった。
    Resolved,
    /// 旋律チャンネルで該当(bank, program)が見つからず、standalone側が`default_patch`へ
    /// フォールバックしている。
    NotFound,
    /// リズムチャンネル（`program`にキット番号が入る）。
    Rhythm,
    /// standaloneのトレイ起動音色エディタがこのチャンネルを編集中。
    Editing,
}

pub struct ProgramInfo {
    pub bank: u16,
    pub program: u8,
    pub name: String,
    pub status: ProgramStatus,
}

/// standaloneへ問い合わせる。接続・応答のいずれかに失敗したら`None`
/// （呼び出し側は「standalone未接続」として扱う）。
pub fn query_program(channel: u8) -> Option<ProgramInfo> {
    let mut pipe = OpenOptions::new().read(true).write(true).open(PIPE_PATH).ok()?;

    let mut request = vec![FRAME_VERSION, REQUEST_KIND_QUERY_PROGRAM, 0];
    request.extend_from_slice(&1u16.to_le_bytes());
    request.push(channel);
    pipe.write_all(&request).ok()?;

    let mut buf = [0u8; 512];
    let n = pipe.read(&mut buf).ok()?;
    decode_response(&buf[..n])
}

fn decode_response(raw: &[u8]) -> Option<ProgramInfo> {
    if raw.len() < 5 {
        return None;
    }
    let version = raw[0];
    let kind = raw[1];
    let len = u16::from_le_bytes([raw[3], raw[4]]) as usize;
    if version != FRAME_VERSION || kind != RESPONSE_KIND_PROGRAM_INFO || len < 5 || raw.len() < 5 + len {
        return None;
    }
    let payload = &raw[5..5 + len];
    let status = match payload[0] {
        0 => ProgramStatus::Resolved,
        1 => ProgramStatus::NotFound,
        2 => ProgramStatus::Rhythm,
        3 => ProgramStatus::Editing,
        _ => return None,
    };
    let bank = u16::from_le_bytes([payload[1], payload[2]]);
    let program = payload[3];
    let name_len = payload[4] as usize;
    if payload.len() < 5 + name_len {
        return None;
    }
    let name = String::from_utf8_lossy(&payload[5..5 + name_len]).into_owned();
    Some(ProgramInfo { bank, program, name, status })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// query_server.rsの`encode_program_info_response`が組み立てるのと同じ形のフレームを
    /// ここで直接組み立てて、デコード側の単体テストとする（2クレートにまたがるため
    /// 実装は共有せず、フレーム形式のドキュメントで同期を取る）。
    fn build_response_frame(status: u8, bank: u16, program: u8, name: &str) -> Vec<u8> {
        let name_bytes = name.as_bytes();
        let mut payload = vec![status];
        payload.extend_from_slice(&bank.to_le_bytes());
        payload.push(program);
        payload.push(name_bytes.len() as u8);
        payload.extend_from_slice(name_bytes);

        let mut frame = vec![FRAME_VERSION, RESPONSE_KIND_PROGRAM_INFO, 0];
        frame.extend_from_slice(&(payload.len() as u16).to_le_bytes());
        frame.extend_from_slice(&payload);
        frame
    }

    #[test]
    fn decode_response_reads_resolved_name() {
        let frame = build_response_frame(0, 3, 7, "TestPatch");
        let info = decode_response(&frame).expect("should decode");
        assert_eq!(info.status, ProgramStatus::Resolved);
        assert_eq!((info.bank, info.program), (3, 7));
        assert_eq!(info.name, "TestPatch");
    }

    #[test]
    fn decode_response_reads_not_found_status() {
        let frame = build_response_frame(1, 99, 1, "");
        let info = decode_response(&frame).expect("should decode");
        assert_eq!(info.status, ProgramStatus::NotFound);
        assert_eq!(info.name, "");
    }

    #[test]
    fn decode_response_rejects_wrong_version() {
        let mut frame = build_response_frame(0, 0, 0, "x");
        frame[0] = FRAME_VERSION + 1;
        assert!(decode_response(&frame).is_none());
    }

    #[test]
    fn decode_response_rejects_wrong_kind() {
        let mut frame = build_response_frame(0, 0, 0, "x");
        frame[1] = REQUEST_KIND_QUERY_PROGRAM;
        assert!(decode_response(&frame).is_none());
    }

    #[test]
    fn decode_response_rejects_truncated_frame() {
        let mut frame = build_response_frame(0, 1, 2, "Name");
        frame.truncate(frame.len() - 1);
        assert!(decode_response(&frame).is_none());
    }

    #[test]
    fn decode_response_rejects_unknown_status() {
        let frame = build_response_frame(0xFF, 0, 0, "");
        assert!(decode_response(&frame).is_none());
    }
}
