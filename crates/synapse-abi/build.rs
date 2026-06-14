// 正本(Rust)から C ヘッダを生成する。`cargo build` で synapse_abi.h を更新。
// 文字列 ABI 定数は cbindgen が出力できないため、正本 src/abi_strings.rs から
// C の #define を生成して after_includes に注入する(これで SSoT を保つ)。

// include! した定数の一部しか使わなくても警告にしない。
#![allow(dead_code)]

include!("src/abi_strings.rs");

fn main() {
    let crate_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let mut config = cbindgen::Config::from_root_or_default(&crate_dir);

    // 不透明ハンドルの前方宣言。Rust 側は Nomicon 流のゼロサイズ struct + PhantomData だが、
    // cbindgen はその形では型を出力しない（PhantomData のみのため）。C 側で不完全型として
    // 使えるよう、前方 typedef をここで注入する（生成本体には現れないので重複しない）。
    let opaque_fwd = concat!(
        "\n/* ---- 不透明ハンドル（不完全型・正本: src/lib.rs の opaque_handle!） ---- */\n",
        "typedef struct SynNode SynNode;\n",
        "typedef struct SynDeclBuilder SynDeclBuilder;\n",
        "typedef struct SynEvalCtx SynEvalCtx;\n",
    );

    let defines = format!(
        concat!(
            "{}",
            "\n/* ---- 文字列 ABI 定数 (正本: src/abi_strings.rs) ---- */\n",
            "#define SYN_TYPE_REGISTRY_SUITE \"{}\"\n",
            "#define SYN_URID_SUITE          \"{}\"\n",
            "#define SYN_DECL_SUITE          \"{}\"\n",
            "#define SYN_EVAL_SUITE          \"{}\"\n",
            "#define SYN_EXT_UI              \"{}\"\n",
            "#define SYN_MODULE_ENTRY_SYMBOL \"{}\"\n",
        ),
        opaque_fwd,
        SYN_TYPE_REGISTRY_SUITE,
        SYN_URID_SUITE,
        SYN_DECL_SUITE,
        SYN_EVAL_SUITE,
        SYN_EXT_UI,
        SYN_MODULE_ENTRY_SYMBOL,
    );
    config.after_includes = Some(match config.after_includes.take() {
        Some(s) => s + &defines,
        None => defines,
    });

    cbindgen::Builder::new()
        .with_crate(&crate_dir)
        .with_config(config)
        .generate()
        .expect("cbindgen failed")
        .write_to_file(format!("{crate_dir}/synapse_abi.h"));

    println!("cargo:rerun-if-changed=src/lib.rs");
    println!("cargo:rerun-if-changed=src/abi_strings.rs");
    println!("cargo:rerun-if-changed=cbindgen.toml");
}
