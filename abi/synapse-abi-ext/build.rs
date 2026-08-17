// 拡張層の C ヘッダを生成する。出力対象は自クレートの src/ から自動導出される。

const BANNER: &str = concat!(
    "/* ============================================================================\n",
    " *  自動生成ファイル — 手で編集しないこと。正本: abi/synapse-abi-ext/src/\n",
    " * ----------------------------------------------------------------------------\n",
    " *  拡張層 — desc->get_extension(instance, ext_id) で取得する。\n",
    " *  未対応なら NULL が返るだけなので、拡張の追加は完全に非破壊。\n",
    " *\n",
    " *  通常は synapse_abi.h（アンブレラ）を include すればよい。\n",
    " * ========================================================================== */\n",
);

fn main() {
    synapse_abi_buildgen::generate(synapse_abi_buildgen::Layer {
        header: "synapse_abi_ext.h",
        guard: "SYNAPSE_ABI_EXT_H",
        banner: BANNER,
        includes: &["synapse_abi_core.h"],
        sys_includes: false,
        after_includes: "",
    });
}
