// スイート層の C ヘッダを生成する。出力対象は自クレートの src/ から自動導出される。

const BANNER: &str = concat!(
    "/* ============================================================================\n",
    " *  自動生成ファイル — 手で編集しないこと。正本: abi/synapse-abi-suite/src/\n",
    " * ----------------------------------------------------------------------------\n",
    " *  スイート層 — host->fetch_suite(id) で取得する。\n",
    " *  未知の id には NULL が返るため、スイートの「追加」は基底 ABI に触れない非破壊変更。\n",
    " *  ただし個々のスイート構造体に版数は無く、フィールド追加は基底同様に破壊的（Open-13）。\n",
    " *\n",
    " *  通常は synapse_abi.h（アンブレラ）を include すればよい。\n",
    " * ========================================================================== */\n",
);

fn main() {
    synapse_abi_buildgen::generate(synapse_abi_buildgen::Layer {
        header: "synapse_abi_suite.h",
        guard: "SYNAPSE_ABI_SUITE_H",
        banner: BANNER,
        includes: &["synapse_abi_core.h"],
        sys_includes: false,
        after_includes: "",
    });
}
