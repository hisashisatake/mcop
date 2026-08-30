//! x86ビルドではstdcallエクスポートが`_DriverProc@20`のように名前修飾される。
//! winmm.dllは修飾なしの名前（`DriverProc`）で探すため、.defファイルでエイリアスを与える。
//! x64には修飾が無いため、別内容の.defを使う（archで切り替え、`/DEF:`をcdylibリンクへ渡す）。

fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").expect("CARGO_CFG_TARGET_ARCH not set");

    let def_name = match arch.as_str() {
        "x86" => "op505mme-x86.def",
        "x86_64" => "op505mme-x64.def",
        other => panic!("op505-mme-driver は x86 / x86_64 の Windows ターゲットのみ対応（arch={other}）"),
    };

    let def_path = std::path::Path::new(&manifest_dir).join(def_name);
    println!("cargo:rerun-if-changed={}", def_path.display());
    println!("cargo:rustc-cdylib-link-arg=/DEF:{}", def_path.display());
}
