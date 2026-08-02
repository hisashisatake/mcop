// ymfm OPZ(C++)とFFI shim をビルドして静的リンクする。
fn main() {
    let mut build = cc::Build::new();
    build
        .cpp(true)
        .std("c++17")
        .include("vendor/ymfm")
        .file("vendor/ymfm/ymfm_opz.cpp")
        .file("csrc/shim.cpp")
        .warnings(false);
    // MSVC は例外処理に /EHsc が必要。GCC/Clang では無視される。
    build.flag_if_supported("/EHsc");
    // ソースを UTF-8 として解釈させる（cp932 誤認による破損を防ぐ）。
    build.flag_if_supported("/utf-8");
    build.compile("opzref_ymfm");

    println!("cargo:rerun-if-changed=csrc/shim.cpp");
    println!("cargo:rerun-if-changed=vendor/ymfm/ymfm_opz.cpp");
    println!("cargo:rerun-if-changed=vendor/ymfm/ymfm_opz.h");
    println!("cargo:rerun-if-changed=vendor/ymfm/ymfm_fm.ipp");
    println!("cargo:rerun-if-changed=vendor/ymfm/ymfm_fm.h");
    println!("cargo:rerun-if-changed=vendor/ymfm/ymfm.h");
}
