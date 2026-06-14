# synapse-abi

Synapse プラグイン C ABI の宣言クレート。**正本(Single Source of Truth)は Rust 側**
(`src/lib.rs` と `src/abi_strings.rs`)で、C ヘッダ `synapse_abi.h` は cbindgen による生成物。

## ファイル構成

| ファイル | 役割 |
| --- | --- |
| `src/lib.rs` | ABI 定義の正本(構造体・関数ポインタ・定数)。宣言のみで実装を持たない |
| `src/abi_strings.rs` | 文字列 ABI 定数(スイートID等)の正本。lib.rs と build.rs の両方から `include!` される |
| `build.rs` | `cargo build` 時に cbindgen でヘッダを再生成。文字列定数を `#define` として注入 |
| `cbindgen.toml` | 生成設定。設計概要コメント(header)と補助関数・評価ループ擬似コード(trailer)もここに置く |
| `synapse_abi.h` | 生成物(コミット対象。プラグイン作者への配布物)。**手で編集しないこと** |
| `../../docs/synapse_adr.md` | 設計判断の記録(ADR)。決定・却下案・ロードマップ・未決事項 |

## 再生成と鮮度確認

```sh
cargo build -p synapse-abi          # synapse_abi.h を再生成
git diff --exit-code crates/synapse-abi/synapse_abi.h   # コミット済みヘッダとの一致確認
```

注意: `cbindgen` CLI 直叩きでは文字列 ABI 定数(`#define SYN_*_SUITE` 等)が出力されない。
完全な生成は必ず `cargo build` 経由で行う。

## 生成ヘッダの受け入れテスト

clang (VS 同梱の LLVM など) で:

```sh
clang -std=c11   -Wall -Wextra -Wpedantic -fsyntax-only -x c   synapse_abi.h
clang -std=c++14 -Wall -Wextra -Wpedantic -fsyntax-only -x c++ synapse_abi.h
# 二重 include ガード確認
printf '#include "synapse_abi.h"\n#include "synapse_abi.h"\nint main(void){return 0;}\n' > t.c
clang -std=c11 -Wall -Wextra -Wpedantic -I. t.c
```

(ヘッダ単体コンパイル時の `-Wunused-function` 警告は static inline 補助関数によるもので無害。)

## 設計メモ

- 不透明ハンドル(`SynNode` 等)は Rust 側で Nomicon 流のゼロサイズ struct +
  `PhantomData<(*mut u8, PhantomPinned)>` として宣言する(uninhabited な空 enum への参照が
  UB になるのを避け、`!Send`/`!Sync`/`!Unpin` も表現する)。cbindgen はこの形では型本体を
  出力しないため、C の前方宣言 `typedef struct X X;`(不完全型)を build.rs の `after_includes`
  で注入している。
- 定数を Rust enum ではなく `pub const` にしているのは、C の enum 基底型が処理系定義で
  ABI 幅を固定できないため(`#define` + 固定幅 typedef に落とす)。
- 設計意図・アーキテクチャの柱・ロードマップは生成ヘッダ冒頭のコメントに、
  決定の経緯と却下案は [../../docs/synapse_adr.md](../../docs/synapse_adr.md) に記録している。
