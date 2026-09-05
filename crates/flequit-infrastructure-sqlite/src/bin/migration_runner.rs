//! テストビルド用マイグレーション実行バイナリ
//!
//! build.rsから呼び出され、指定されたパスにSQLiteデータベースを作成し、
//! マイグレーションを実行する。

use flequit_infrastructure_sqlite::infrastructure::database_manager::DatabaseManager;
use sea_orm_migration::MigratorTrait;
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 || args.len() > 3 {
        eprintln!("Usage: migration_runner <database_path> [--force]");
        std::process::exit(1);
    }

    let db_path = &args[1];
    let force_mode = args.len() == 3 && args[2] == "--force";

    if force_mode {
        println!(
            "🔄 強制マイグレーション実行開始（全テーブル削除→再作成）: {}",
            db_path
        );
    } else {
        println!("🔧 マイグレーション実行開始: {}", db_path);
    }

    // 環境変数でデータベースパスを指定
    //
    // SAFETY: この時点ではまだスレッドを起動していない単一スレッド実行のため、
    // 他スレッドから環境変数を読み書きされる可能性がない。
    // edition 2024 では `set_var` が unsafe になったため明示する。
    unsafe {
        env::set_var("FLEQUIT_DB_PATH", db_path);
    }

    // 強制モードの場合は、既存のDBファイルを削除
    if force_mode && std::path::Path::new(db_path).exists() {
        println!("⚠️  既存のデータベースファイルを削除します");
        std::fs::remove_file(db_path)?;
    }

    // DatabaseManagerを作成（シングルトンではない新しいインスタンスが必要）
    let db_manager = DatabaseManager::new_for_test(db_path);
    let db = db_manager.get_connection().await?;

    // マイグレーション実行（通常モード・強制モード共に同じ処理）
    // 強制モードの場合はファイル削除済みなので、新規作成として実行される
    flequit_infrastructure_sqlite::migrator::Migrator::up(db, None).await?;

    println!("✅ マイグレーション完了: {}", db_path);

    Ok(())
}
