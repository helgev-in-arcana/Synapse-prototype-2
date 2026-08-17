// 基底 ABI 層の C ヘッダを生成する。出力対象は自クレートの src/ から自動導出される
// （手書きリストは無い）。詳細は synapse-abi-buildgen を参照。

/// 不透明ハンドルの前方宣言。
///
/// Rust 側は `opaque_handle!` マクロ生成の PhantomData 構造体で、cbindgen は型本体を
/// 出力しない（syn からも `Item::Macro` に見えるので導出リストにも入らない）。
/// C 側で不完全型として使えるよう、ここで前方 typedef を注入する。
const OPAQUE_FWD: &str = concat!(
    "\n/* ---- 不透明ハンドル（不完全型・正本: src/handle.rs の opaque_handle!） ---- */\n",
    "typedef struct SynNode SynNode;\n",
    "typedef struct SynDeclBuilder SynDeclBuilder;\n",
    "typedef struct SynEvalCtx SynEvalCtx;\n",
);

const BANNER: &str = concat!(
    "/* ============================================================================\n",
    " *  自動生成ファイル — 手で編集しないこと。正本: abi/synapse-abi-core/src/\n",
    " * ----------------------------------------------------------------------------\n",
    " *  基底 ABI 層 — 版一致必須の凍結面。\n",
    " *  ここの型レイアウトを変えることは破壊的変更であり SYN_ABI_VERSION の更新を要する\n",
    " *  （ホストは != で拒否する。ADR-020）。この層は suite/ext を参照しない。\n",
    " *\n",
    " *  通常は synapse_abi.h（アンブレラ）を include すればよい。\n",
    " * ========================================================================== */\n",
);

fn main() {
    synapse_abi_buildgen::generate(synapse_abi_buildgen::Layer {
        header: "synapse_abi_core.h",
        guard: "SYNAPSE_ABI_CORE_H",
        banner: BANNER,
        includes: &[],
        sys_includes: true,
        after_includes: OPAQUE_FWD,
    });
}
