//! Opening a store that has to be there: `SqliteStore::open_existing` refuses a missing file, naming the path, and
//! makes neither the file nor its directory; once the file is there it opens it as `open` does.

use std::path::PathBuf;

use chidev_core::app::ports::Store;
use chidev_core::domain::repository::StoreError;
use chidev_core::infra::sqlite::SqliteStore;

/// A directory of its own that is not there.
fn fresh(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("chidev-store-open-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[tokio::test]
async fn a_missing_store_is_refused_with_its_path_and_not_made() {
    let dir = fresh("missing-dir");
    let path = dir.join("nested").join("chidev.db");
    let Err(error) = SqliteStore::open_existing(&path).await else {
        panic!("a store that is not there was opened");
    };
    // the caller has the path and says it; the error is about what is wrong with it
    assert_eq!(
        error,
        StoreError::Failed("there is no store file".to_string())
    );
    assert!(!dir.exists(), "the directory was made");

    // the directory is there and the file is not
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("chidev.db");
    assert!(SqliteStore::open_existing(&path).await.is_err());
    assert!(!path.exists(), "the file was made");
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
}

/// A path whose metadata cannot be read is refused with the system's message, not as a store that is not there.
#[cfg(unix)]
#[tokio::test]
async fn a_path_that_cannot_be_read_is_refused_with_the_system_s_message() {
    use std::os::unix::fs::PermissionsExt;

    let dir = fresh("unreadable");
    let path = dir.join("held").join("chidev.db");
    let held = dir.join("held");
    std::fs::create_dir_all(&held).unwrap();
    std::fs::write(&path, b"").unwrap();
    std::fs::set_permissions(&held, std::fs::Permissions::from_mode(0o000)).unwrap();
    let readable = std::fs::metadata(&path).is_ok(); // a privileged user reads it all the same
    let result = SqliteStore::open_existing(&path).await;
    std::fs::set_permissions(&held, std::fs::Permissions::from_mode(0o755)).unwrap();
    if readable {
        return;
    }
    let Err(StoreError::Failed(message)) = result else {
        panic!("a path that cannot be read was opened or failed otherwise");
    };
    assert_ne!(message, "there is no store file");
    assert!(message.contains("Permission denied"), "{message}");
}

#[tokio::test]
async fn a_directory_is_no_store() {
    let dir = fresh("directory");
    std::fs::create_dir_all(&dir).unwrap();
    assert!(SqliteStore::open_existing(&dir).await.is_err());
}

#[tokio::test]
async fn a_store_that_is_there_is_opened_with_what_it_holds() {
    let dir = fresh("there");
    let path = dir.join("chidev.db");
    let made = SqliteStore::open(&path).await.unwrap();
    let revision = made.revision().await.unwrap();
    made.close().await.unwrap();

    let again = SqliteStore::open_existing(&path).await.unwrap();
    assert_eq!(again.revision().await.unwrap(), revision);
    again.close().await.unwrap();
}
