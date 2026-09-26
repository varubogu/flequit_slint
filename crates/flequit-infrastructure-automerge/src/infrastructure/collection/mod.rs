//! エンティティの集合を Automerge ドキュメントに置く形
//!
//! 集合はドキュメント直下の 1 つのキーに「エンティティのキー → エンティティ」の Map として置く。
//! Automerge が推奨する形で、次の性質がある。
//!
//! - 1 件の保存は、そのエンティティの変わったフィールドだけを書く
//! - 別々の端末で別々のエンティティ（同じエンティティの別々のフィールドも）を同時に更新しても、
//!   マージ後に両方が残る。リストを丸ごと置き換える形では、同時に置かれた 2 つのリストの
//!   どちらか一方しか残らない
//!
//! 以前は集合をリストで保存していた。読み取りはどちらの形も受け付け、書き込むときに
//! その場で Map に変換する（[`legacy`]）。

mod access;
mod legacy;
#[cfg(test)]
mod tests;

/// ドキュメント内のエンティティの集合
///
/// `name` はドキュメント直下のキー、`key_of` は集合の中でエンティティを識別するキー。
/// キーは同じエンティティに対して常に同じ値を返すこと（通常は ID）。
pub struct Collection<T> {
    name: &'static str,
    key_of: fn(&T) -> String,
}

impl<T> Collection<T> {
    pub const fn new(name: &'static str, key_of: fn(&T) -> String) -> Self {
        Self { name, key_of }
    }

    /// ドキュメント直下のキー
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// `entity` を集合の中で識別するキー
    pub fn key_of(&self, entity: &T) -> String {
        (self.key_of)(entity)
    }
}

/// 2 つの ID の組で識別する関連（タスクとタグなど）のキー
pub fn relation_key(parent: &str, child: &str) -> String {
    format!("{parent}:{child}")
}
