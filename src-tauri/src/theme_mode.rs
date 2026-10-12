//! Normal-user settings step, after the privileged skin transaction and restart.
//! Persist intent before UAC/RPC, so retries never lose the first original mode.
use crate::{
    ime_rpc::{self, ThemeMode},
    safe_fs, skin,
    transaction::Store,
    workdir,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    schema: u32,
    version: String,
    original: ThemeMode,
    job: String,
    pending: bool,
    managed: bool,
}
fn path(version: &str) -> Result<PathBuf, String> {
    if !skin::valid_ime_version(version) {
        return Err("无效的模式记录版本".into());
    }
    Ok(workdir::app_dir().join(format!("theme-mode-{version}.json")))
}
fn read(path: &Path, version: &str) -> Result<Option<Record>, String> {
    if !path.try_exists().map_err(|e| e.to_string())? {
        return Ok(None);
    }
    decode_record(&safe_fs::read(path, 8192)?, version).map(Some)
}
fn decode_record(bytes: &[u8], version: &str) -> Result<Record, String> {
    let record: Record =
        serde_json::from_slice(bytes).map_err(|e| format!("外观模式恢复记录损坏：{e}"))?;
    if record.schema != 1 || record.version != version {
        return Err("外观模式记录不匹配".into());
    }
    Ok(record)
}
pub(crate) fn validate_migration_record(bytes: &[u8], version: &str) -> Result<(), String> {
    decode_record(bytes, version).map(|_| ())
}
fn save(path: &Path, record: &Record) -> Result<(), String> {
    safe_fs::atomic_write(
        path,
        &serde_json::to_vec(record).map_err(|e| e.to_string())?,
    )
}
fn remove(path: &Path) -> Result<(), String> {
    let _guards = safe_fs::guard_path(path)?;
    if path.try_exists().map_err(|e| e.to_string())? {
        std::fs::remove_file(path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub struct Prepared {
    path: PathBuf,
    previous: Option<Record>,
}
impl Prepared {
    /// Only when Windows confirms that the helper was never launched.
    pub fn cancel(self) -> Result<(), String> {
        match self.previous {
            Some(record) => save(&self.path, &record),
            None => remove(&self.path),
        }
    }
}
pub fn prepare(job: &skin::Job) -> Result<Option<Prepared>, String> {
    if !matches!(job.action, skin::Action::Apply)
        || skin::layout_for_hashes(&job.expected)? != skin::Layout::SplitDark
    {
        return Ok(None);
    }
    let current = ime_rpc::theme_mode()?;
    let path = path(&job.version)?;
    let target = skin::DetectedIme {
        version: job.version.clone(),
        skin: skin::job_skin(job)?,
    };
    let manifest = skin::store_for(&target)?.manifest()?;
    workdir::ensure_app_dir()?;
    prepare_at(
        path,
        &job.version,
        &job.id,
        current,
        manifest.as_ref().map(|m| m.job.as_str()),
    )
    .map(Some)
}
fn prepare_at(
    path: PathBuf,
    version: &str,
    job: &str,
    current: ThemeMode,
    committed_job: Option<&str>,
) -> Result<Prepared, String> {
    let previous = read(&path, version)?;
    let previous_committed = previous
        .as_ref()
        .is_some_and(|r| r.managed || committed_job == Some(r.job.as_str()));
    let original = previous
        .as_ref()
        .filter(|_| previous_committed)
        .map_or(current, |r| r.original);
    let record = Record {
        schema: 1,
        version: version.into(),
        original,
        job: job.into(),
        pending: true,
        managed: previous.as_ref().is_some_and(|r| r.managed),
    };
    save(&path, &record)?;
    Ok(Prepared { path, previous })
}

trait ModeSettings {
    fn set(&mut self, next: ThemeMode, expected: Option<ThemeMode>) -> Result<(), String>;
}
struct LiveSettings;
impl ModeSettings for LiveSettings {
    fn set(&mut self, next: ThemeMode, expected: Option<ThemeMode>) -> Result<(), String> {
        ime_rpc::set_theme_mode(next, expected)
    }
}
fn finish_at(
    path: &Path,
    version: &str,
    store: &Store,
    settings: &mut impl ModeSettings,
) -> Result<(), String> {
    let Some(mut record) = read(path, version)? else {
        return Ok(());
    };
    if store.recovery_needed()? {
        return Err("皮肤事务尚未恢复，未切换官方外观模式".into());
    }
    let live = store.live_hashes()?;
    let originals = Store::hashes(&store.originals()?);
    let manifest = store.manifest()?;
    if manifest.as_ref().is_some_and(|m| m.applied != live) {
        return Err("皮肤在模式切换前发生变化，已停止".into());
    }
    if live == originals {
        if record.managed {
            settings.set(record.original, Some(ThemeMode::Dark))?;
        }
        return remove(path);
    }
    if record.pending && manifest.as_ref().is_some_and(|m| m.job == record.job) {
        // Mark ownership before the RPC: a timeout may mean the write succeeded.
        record.managed = true;
        save(path, &record)?;
        settings.set(ThemeMode::Dark, None)?;
        record.pending = false;
        save(path, &record)?;
    } else if record.pending {
        // A cancelled/rolled-back transaction must never trigger a mode change.
        record.pending = false;
        if record.managed {
            save(path, &record)?;
        } else {
            remove(path)?;
        }
    }
    Ok(())
}
pub fn finish(target: &skin::DetectedIme) -> Result<(), String> {
    if workdir::review_mode() {
        return Err("检查模式不修改官方设置".into());
    }
    if skin::layout(&target.skin) != skin::Layout::SplitDark {
        return Ok(());
    }
    finish_at(
        &path(&target.version)?,
        &target.version,
        &skin::store_for(target)?,
        &mut LiveSettings,
    )
    .map_err(|e| format!("皮肤文件操作已完成，但官方外观模式未同步：{e}；可再次安装或卸载重试"))
}
pub fn pending(version: &str, skin_applied: bool) -> Result<bool, String> {
    Ok(read(&path(version)?, version)?.is_some_and(|r| r.pending || (!skin_applied && r.managed)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transaction::Files;
    struct Fake {
        mode: ThemeMode,
        calls: usize,
        fail_after_write: bool,
    }
    impl ModeSettings for Fake {
        fn set(&mut self, next: ThemeMode, expected: Option<ThemeMode>) -> Result<(), String> {
            self.calls += 1;
            if expected.is_none_or(|v| v == self.mode) {
                self.mode = next;
            }
            if self.fail_after_write {
                Err("timeout".into())
            } else {
                Ok(())
            }
        }
    }
    struct Fixture {
        root: PathBuf,
        record: PathBuf,
        store: Store,
        original: Files,
        next: Files,
    }
    impl Fixture {
        fn new() -> Self {
            let root = workdir::new_scratch().unwrap();
            let record = root.join("mode.json");
            let skin = root.join("skin");
            let original: Files = [("dark/file.svg".into(), b"original".to_vec())].into();
            let next: Files = [("dark/file.svg".into(), b"themed".to_vec())].into();
            safe_fs::atomic_write(&skin.join("dark/file.svg"), b"original").unwrap();
            let store = Store::new(skin, "vtest".into(), Store::hashes(&original));
            save(
                &record,
                &Record {
                    schema: 1,
                    version: "vtest".into(),
                    original: ThemeMode::System,
                    job: "install".into(),
                    pending: true,
                    managed: false,
                },
            )
            .unwrap();
            Self {
                root,
                record,
                store,
                original,
                next,
            }
        }
        fn apply(&self) {
            self.store
                .apply(
                    &self.next,
                    &self.store.live_hashes().unwrap(),
                    "test",
                    "install",
                )
                .unwrap();
        }
        fn restore(&self) {
            self.store
                .apply(
                    &self.original,
                    &self.store.live_hashes().unwrap(),
                    "official",
                    "restore",
                )
                .unwrap();
        }
        fn finish(&self, settings: &mut Fake) -> Result<(), String> {
            finish_at(&self.record, "vtest", &self.store, settings)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.root).unwrap();
        }
    }
    #[test]
    fn reinstall_retains_first_original_and_uac_cancel_restores_previous_record() {
        let f = Fixture::new();
        let mut rpc = Fake {
            mode: ThemeMode::System,
            calls: 0,
            fail_after_write: false,
        };
        f.apply();
        f.finish(&mut rpc).unwrap();
        let prepared = prepare_at(
            f.record.clone(),
            "vtest",
            "second",
            ThemeMode::Dark,
            Some("install"),
        )
        .unwrap();
        assert_eq!(
            read(&f.record, "vtest").unwrap().unwrap().original,
            ThemeMode::System
        );
        prepared.cancel().unwrap();
        let previous = read(&f.record, "vtest").unwrap().unwrap();
        assert_eq!(previous.job, "install");
        assert!(!previous.pending);
        prepare_at(
            f.record.clone(),
            "vtest",
            "second",
            ThemeMode::Dark,
            Some("install"),
        )
        .unwrap();
        f.store
            .apply(&f.next, &f.store.live_hashes().unwrap(), "test", "second")
            .unwrap();
        f.finish(&mut rpc).unwrap();
        f.restore();
        f.finish(&mut rpc).unwrap();
        assert_eq!(rpc.mode, ThemeMode::System);
    }
    #[test]
    fn abandoned_uncommitted_intent_does_not_reuse_a_stale_original() {
        let f = Fixture::new();
        prepare_at(f.record.clone(), "vtest", "retry", ThemeMode::Light, None).unwrap();
        assert_eq!(
            read(&f.record, "vtest").unwrap().unwrap().original,
            ThemeMode::Light
        );
    }
    #[test]
    fn commit_switches_dark_and_restore_recovers_first_mode() {
        let f = Fixture::new();
        let mut rpc = Fake {
            mode: ThemeMode::System,
            calls: 0,
            fail_after_write: false,
        };
        f.apply();
        f.finish(&mut rpc).unwrap();
        assert_eq!(rpc.mode, ThemeMode::Dark);
        f.finish(&mut rpc).unwrap();
        assert_eq!(rpc.calls, 1);
        f.restore();
        f.finish(&mut rpc).unwrap();
        assert_eq!(rpc.mode, ThemeMode::System);
        assert!(!f.record.exists());
    }
    #[test]
    fn cancellation_and_user_later_mode_are_preserved() {
        let f = Fixture::new();
        let mut rpc = Fake {
            mode: ThemeMode::Light,
            calls: 0,
            fail_after_write: false,
        };
        f.finish(&mut rpc).unwrap();
        assert_eq!(rpc.calls, 0);
        assert!(!f.record.exists());
        let f = Fixture::new();
        f.apply();
        f.finish(&mut rpc).unwrap();
        rpc.mode = ThemeMode::Light;
        f.restore();
        f.finish(&mut rpc).unwrap();
        assert_eq!(rpc.mode, ThemeMode::Light);
    }
    #[test]
    fn ambiguous_rpc_failure_is_retryable_and_restorable() {
        let f = Fixture::new();
        let mut rpc = Fake {
            mode: ThemeMode::System,
            calls: 0,
            fail_after_write: true,
        };
        f.apply();
        assert!(f.finish(&mut rpc).is_err());
        let record = read(&f.record, "vtest").unwrap().unwrap();
        assert!(record.pending && record.managed);
        rpc.fail_after_write = false;
        f.finish(&mut rpc).unwrap();
        f.restore();
        rpc.fail_after_write = true;
        assert!(f.finish(&mut rpc).is_err());
        assert!(f.record.exists());
        rpc.fail_after_write = false;
        f.finish(&mut rpc).unwrap();
        assert_eq!(rpc.mode, ThemeMode::System);
        assert!(!f.record.exists());
    }
}
