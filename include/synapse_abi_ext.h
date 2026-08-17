/* ============================================================================
 *  自動生成ファイル — 手で編集しないこと。正本: abi/synapse-abi-ext/src/
 * ----------------------------------------------------------------------------
 *  拡張層 — desc->get_extension(instance, ext_id) で取得する。
 *  未対応なら NULL が返るだけなので、拡張の追加は完全に非破壊。
 *
 *  通常は synapse_abi.h（アンブレラ）を include すればよい。
 * ========================================================================== */


#ifndef SYNAPSE_ABI_EXT_H
#define SYNAPSE_ABI_EXT_H


#include "synapse_abi_core.h"

/* ---- 文字列 ABI 定数 (正本: このクレートの Rust ソース) ---- */
#define SYN_EXT_UI "synapse:ext:ui"


/*
 UI コンポーネント拡張。
 */
typedef struct SynUiExt {
  /*
   UI を構築する（host_ui_handle はホスト側 UI コンテキスト）。
   */
  SynStatus (*build)(void *instance, void *host_ui_handle);
  /*
   パラメータ変更の通知（param_key は変更されたソケット key）。
   */
  SynStatus (*on_change)(void *instance, const char *param_key);
} SynUiExt;

#endif  /* SYNAPSE_ABI_EXT_H */
