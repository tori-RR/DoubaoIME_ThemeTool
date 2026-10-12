use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

pub const APP_DIR: &str = r"C:\tmp\DoubaoIME_TT";
const LEGACY_APP_DIR: &str = r"C:\tmp\DoubaoIME Darkmode";
const STATE_FILE: &str = "DoubaoIME Darkmode.json";
const MIGRATION_MARKER: &str = "migration-darkmode-v1.complete";
static MIGRATION: OnceLock<Result<(), String>> = OnceLock::new();

/// Review mode never imports personal state or touches either real directory.
pub fn migrate_legacy_dir(
    validate_state: impl Fn(&[u8]) -> Result<(), String>,
) -> Result<(), String> {
    let result = MIGRATION.get_or_init(|| {
        if review_mode() {
            return Ok(());
        }
        migrate_at(
            Path::new(LEGACY_APP_DIR),
            &ensure_app_dir()?,
            validate_state,
        )
        .map_err(|err| {
            format!("旧配置迁移未完成，已暂停保存和安装：{err}；旧目录已保留，请关闭旧版助手后重试")
        })
    });
    result.clone()
}

pub fn check_migration() -> Result<(), String> {
    MIGRATION.get().cloned().unwrap_or(Ok(()))
}

fn migrate_at(
    source: &Path,
    destination: &Path,
    validate_state: impl Fn(&[u8]) -> Result<(), String>,
) -> Result<(), String> {
    use crate::{avatar, safe_fs, skin, theme_mode};
    let _destination_guard = safe_fs::guard_path(destination)?;
    fs::create_dir_all(destination).map_err(|e| e.to_string())?;
    let _destination_lock = safe_fs::exclusive_lock(&destination.join(".state.lock"))?;
    let marker = destination.join(MIGRATION_MARKER);
    if marker.try_exists().map_err(|e| e.to_string())? {
        return if safe_fs::read(&marker, 1)? == b"1" {
            Ok(())
        } else {
            Err("迁移完成标记无效".into())
        };
    }
    let _source_guard = safe_fs::guard_path(source)?;
    if !source.try_exists().map_err(|e| e.to_string())? {
        safe_fs::atomic_write_new(&marker, b"1")?;
        return Ok(());
    }
    // Older assistants use the same lock: do not copy midway through a save.
    let _source_lock = safe_fs::exclusive_lock(&source.join(".state.lock"))?;
    let mut files = Vec::new();
    for entry in fs::read_dir(source).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let mode_version = name
            .strip_prefix("theme-mode-")
            .and_then(|n| n.strip_suffix(".json"));
        let limit = match name {
            STATE_FILE | "DoubaoIME Darkmode.json.bak" => 1024 * 1024,
            "logo.png" => avatar::MAX_UPLOAD_BYTES as u64,
            "ime-restart.pending.json" => 4096,
            "taskbar-refresh.pending" => 1,
            _ if mode_version.is_some_and(skin::valid_ime_version) => 8192,
            _ => continue, // Never copy WebView, watch data or arbitrary files.
        };
        if destination
            .join(name)
            .try_exists()
            .map_err(|e| e.to_string())?
        {
            continue;
        }
        let bytes = safe_fs::read(&path, limit)?;
        match name {
            STATE_FILE | "DoubaoIME Darkmode.json.bak" => validate_state(&bytes)?,
            "logo.png" => {
                avatar::square_pad_png(&bytes)?;
            }
            "ime-restart.pending.json" => {
                let value: serde_json::Value =
                    serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
                if !value
                    .get("version")
                    .and_then(|v| v.as_str())
                    .is_some_and(skin::valid_ime_version)
                {
                    return Err("输入法重启记录无效".into());
                }
            }
            "taskbar-refresh.pending" if bytes == b"1" => {}
            "taskbar-refresh.pending" => return Err("任务栏刷新记录无效".into()),
            _ => theme_mode::validate_migration_record(
                &bytes,
                mode_version.ok_or("模式记录名称无效")?,
            )?,
        }
        stage_file(&mut files, PathBuf::from(name), bytes)?;
    }
    let icons = source.join("taskbar-icons");
    if icons.try_exists().map_err(|e| e.to_string())? {
        let _icons_guard = safe_fs::guard_path(&icons)?;
        for entry in fs::read_dir(&icons).map_err(|e| e.to_string())? {
            let path = entry.map_err(|e| e.to_string())?.path();
            let Some(revision) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if path.extension().and_then(|s| s.to_str()) != Some("png")
                || revision.len() != 64
                || !revision.bytes().all(|b| b.is_ascii_hexdigit())
            {
                continue;
            }
            let relative = Path::new("taskbar-icons").join(path.file_name().ok_or("图标名称无效")?);
            if destination
                .join(&relative)
                .try_exists()
                .map_err(|e| e.to_string())?
            {
                continue;
            }
            let bytes = safe_fs::read(&path, avatar::MAX_STORED_BYTES as u64)?;
            avatar::validate_stored(&bytes)?;
            if safe_fs::hash(&bytes) != revision {
                return Err("任务栏图标草稿哈希不匹配".into());
            }
            stage_file(&mut files, relative, bytes)?;
        }
    }
    // Validate the whole allowlist before publishing any personal state.
    for (relative, bytes) in files {
        safe_fs::atomic_write_new(&destination.join(relative), &bytes)?;
    }
    // Never resurrect a deleted avatar/restore record on subsequent launches.
    safe_fs::atomic_write_new(&marker, b"1")?;
    Ok(())
}

fn stage_file(
    files: &mut Vec<(PathBuf, Vec<u8>)>,
    relative: PathBuf,
    bytes: Vec<u8>,
) -> Result<(), String> {
    if files.len() >= 512
        || files.iter().map(|(_, bytes)| bytes.len()).sum::<usize>() + bytes.len()
            > 64 * 1024 * 1024
    {
        return Err("迁移文件超出预算".into());
    }
    files.push((relative, bytes));
    Ok(())
}
pub fn review_mode() -> bool {
    std::env::args().any(|a| a == "--review-mode")
}
pub fn app_dir() -> PathBuf {
    if review_mode() {
        std::env::temp_dir().join(format!("dmdm-review-{}", std::process::id()))
    } else {
        PathBuf::from(APP_DIR)
    }
}
pub fn ensure_app_dir() -> Result<PathBuf, String> {
    let dir = app_dir();
    let _guard = crate::safe_fs::guard_path(&dir)?;
    fs::create_dir_all(&dir).map_err(|e| format!("无法创建工作目录：{e}"))?;
    Ok(dir)
}
pub fn state_path() -> PathBuf {
    app_dir().join(STATE_FILE)
}
pub fn user_logo_path() -> PathBuf {
    app_dir().join("logo.png")
}

fn restart_pending_path() -> PathBuf {
    app_dir().join("ime-restart.pending.json")
}

pub fn write_restart_pending(version: &str) -> Result<(), String> {
    ensure_app_dir()?;
    let raw = serde_json::to_vec(&serde_json::json!({ "version": version }))
        .map_err(|e| e.to_string())?;
    crate::safe_fs::atomic_write(&restart_pending_path(), &raw)
}

pub fn restart_pending_version() -> Option<String> {
    let raw = crate::safe_fs::read(&restart_pending_path(), 4096).ok()?;
    serde_json::from_slice::<serde_json::Value>(&raw)
        .ok()?
        .get("version")?
        .as_str()
        .filter(|version| version.starts_with('v') && version.len() > 1)
        .map(str::to_string)
}

pub fn clear_restart_pending() {
    let _ = fs::remove_file(restart_pending_path());
}

fn shell_refresh_path() -> PathBuf {
    app_dir().join("taskbar-refresh.pending")
}

pub fn shell_refresh_pending() -> bool {
    shell_refresh_path().exists()
}

pub fn write_shell_refresh_pending() -> Result<(), String> {
    ensure_app_dir()?;
    crate::safe_fs::atomic_write(&shell_refresh_path(), b"1")
}

pub fn clear_shell_refresh_pending() {
    let _ = fs::remove_file(shell_refresh_path());
}
pub fn new_scratch() -> Result<PathBuf, String> {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "dmdm-run-{}-{stamp}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let _guard = crate::safe_fs::guard_path(&dir)?;
    fs::create_dir(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

#[cfg(test)]
mod migration_tests {
    use super::*;
    use crate::safe_fs;

    struct Fixture {
        root: PathBuf,
        old: PathBuf,
        new: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let root = new_scratch().unwrap();
            let old = root.join("old");
            let new = root.join("new");
            fs::create_dir(&old).unwrap();
            Self { root, old, new }
        }
        fn migrate(&self) -> Result<(), String> {
            migrate_at(&self.old, &self.new, |raw| {
                serde_json::from_slice::<serde_json::Value>(raw)
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            })
        }
        fn put(&self, name: &str, bytes: &[u8]) {
            safe_fs::atomic_write(&self.old.join(name), bytes).unwrap();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
    fn png() -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image::RgbaImage::new(16, 16))
            .write_to(&mut out, image::ImageFormat::Png)
            .unwrap();
        out.into_inner()
    }
    const MODE: &[u8] = br#"{"schema":1,"version":"v0.9.2.7","original":"light","job":"test","pending":false,"managed":true}"#;

    #[test]
    fn migrates_state_assets_and_restore_records_not_webview() {
        let f = Fixture::new();
        let png = png();
        let icon = format!("taskbar-icons/{}.png", safe_fs::hash(&png));
        for (name, bytes) in [
            (STATE_FILE, b"{\"theme_id\":\"dark\"}".as_slice()),
            ("DoubaoIME Darkmode.json.bak", b"{}".as_slice()),
            ("logo.png", png.as_slice()),
            (icon.as_str(), png.as_slice()),
            ("theme-mode-v0.9.2.7.json", MODE),
            (
                "ime-restart.pending.json",
                b"{\"version\":\"v0.9.2.7\"}".as_slice(),
            ),
            ("taskbar-refresh.pending", b"1".as_slice()),
        ] {
            f.put(name, bytes);
        }
        f.put("webview/personal-cache", b"do not copy");
        f.put("watch/log", b"do not copy");
        f.migrate().unwrap();
        for name in [
            STATE_FILE,
            "DoubaoIME Darkmode.json.bak",
            "logo.png",
            icon.as_str(),
            "theme-mode-v0.9.2.7.json",
            "ime-restart.pending.json",
            "taskbar-refresh.pending",
        ] {
            assert_eq!(
                fs::read(f.old.join(name)).unwrap(),
                fs::read(f.new.join(name)).unwrap()
            );
        }
        assert!(!f.new.join("webview").exists());
        assert!(!f.new.join("watch").exists());
    }
    #[test]
    fn preserves_existing_destination_and_does_not_resurrect_deleted_records() {
        let f = Fixture::new();
        f.put(STATE_FILE, b"{}");
        f.put("theme-mode-v0.9.2.7.json", MODE);
        fs::create_dir(&f.new).unwrap();
        safe_fs::atomic_write(&f.new.join(STATE_FILE), b"{\"new\":true}").unwrap();
        f.migrate().unwrap();
        assert_eq!(fs::read(f.new.join(STATE_FILE)).unwrap(), b"{\"new\":true}");
        fs::remove_file(f.new.join("theme-mode-v0.9.2.7.json")).unwrap();
        f.migrate().unwrap();
        assert!(!f.new.join("theme-mode-v0.9.2.7.json").exists());
    }
    #[test]
    fn invalid_record_publishes_neither_state_nor_completion() {
        let f = Fixture::new();
        f.put(STATE_FILE, b"{}");
        f.put("theme-mode-v0.9.2.7.json", b"{}");
        assert!(f.migrate().is_err());
        assert!(!f.new.join(STATE_FILE).exists());
        assert!(!f.new.join(MIGRATION_MARKER).exists());
        f.put("theme-mode-v0.9.2.7.json", MODE);
        f.migrate().unwrap();
    }
    #[test]
    fn invalid_json_and_icon_hash_fail_closed() {
        let f = Fixture::new();
        f.put(STATE_FILE, b"broken");
        assert!(f.migrate().is_err());
        f.put(STATE_FILE, b"{}");
        f.put(&format!("taskbar-icons/{}.png", "0".repeat(64)), &png());
        assert!(f.migrate().is_err());
        assert!(!f.new.join(STATE_FILE).exists());
    }
    #[test]
    fn old_writer_lock_blocks_migration_and_retry_succeeds() {
        let f = Fixture::new();
        f.put(STATE_FILE, b"{}");
        let lock = safe_fs::exclusive_lock(&f.old.join(".state.lock")).unwrap();
        assert!(f.migrate().is_err());
        assert!(!f.new.join(MIGRATION_MARKER).exists());
        drop(lock);
        f.migrate().unwrap();
    }
    #[test]
    fn atomic_new_never_overwrites_an_existing_file() {
        let f = Fixture::new();
        let path = f.old.join("new-file");
        assert!(safe_fs::atomic_write_new(&path, b"first").unwrap());
        assert!(!safe_fs::atomic_write_new(&path, b"second").unwrap());
        assert_eq!(fs::read(path).unwrap(), b"first");
    }
    #[test]
    fn rejects_directory_and_oversize_avatar_without_completion() {
        let f = Fixture::new();
        fs::create_dir(f.old.join("logo.png")).unwrap();
        assert!(f.migrate().is_err());
        fs::remove_dir(f.old.join("logo.png")).unwrap();
        let file = fs::File::create(f.old.join("logo.png")).unwrap();
        file.set_len(crate::avatar::MAX_UPLOAD_BYTES as u64 + 1)
            .unwrap();
        drop(file);
        assert!(f.migrate().is_err());
        assert!(!f.new.join(MIGRATION_MARKER).exists());
    }
}
