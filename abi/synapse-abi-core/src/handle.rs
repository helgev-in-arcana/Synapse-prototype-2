//! 不透明ハンドル（すべてホスト所有）。
//!
//! ハンドルは基底層に置く。これらを引数に取る操作群（decl / eval スイート）は
//! suite 層にあり、依存は **suite → core の一方向**になる。

use core::marker::{PhantomData, PhantomPinned};

// 不透明ハンドルは「中身を見せない不完全型へのポインタ」として C へ渡す。
// 空 enum は uninhabited なので、それへの参照（`&SynNode`）を作ると即 UB になる
// （C 側から渡るポインタはハンドルとして有効だが Rust 的には居住者ゼロの型のため）。
// Nomicon 推奨のゼロサイズ struct + PhantomData 方式にすると、不完全型としての性質に加え
// `!Send`/`!Sync`/`!Unpin`（生ポインタと PhantomPinned に由来）も表現でき、cbindgen も
// 不完全型 `typedef struct Foo Foo;` に落とす。
macro_rules! opaque_handle {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[repr(C)]
        pub struct $name {
            _data: [u8; 0],
            _marker: PhantomData<(*mut u8, PhantomPinned)>,
        }
    };
}

opaque_handle!(
    /// ノードインスタンス側ハンドル（不透明）。
    SynNode
);
opaque_handle!(
    /// 宣言ビルダ（不透明）。
    SynDeclBuilder
);
opaque_handle!(
    /// 1 評価分の評価コンテキスト（不透明）。
    SynEvalCtx
);
