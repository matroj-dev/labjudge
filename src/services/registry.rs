use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock, RwLockReadGuard};

use crate::contest::model::{PackageError, ValidatedProblem};
use crate::contest::roster::Roster;
use crate::contest::validate::{is_valid_contest_id, load_contest_draft, validate_problem};
use crate::store::ContestStore;

#[derive(Debug, Clone)]
pub struct ContestPackage {
    pub id: String,
    pub dir: PathBuf,
    pub title: String,
    pub duration_minutes: u32,
    pub leaderboard_visible: bool,
    pub problems: Vec<ValidatedProblem>,
    pub roster: Roster,
}

impl ContestPackage {
    pub fn find_problem(&self, key: &str) -> Option<&ValidatedProblem> {
        self.problems.iter().find(|problem| problem.key == key)
    }
}

#[derive(Debug, Clone)]
pub struct InvalidContest {
    pub id: String,
    pub path: PathBuf,
    pub reason: String,
}

#[derive(Debug, Default)]
pub struct ScanReport {
    pub valid: Vec<Arc<ContestPackage>>,
    pub invalid: Vec<InvalidContest>,
}

pub fn scan_contests(contests_dir: &Path) -> ScanReport {
    let mut report = ScanReport::default();
    let entries = match fs::read_dir(contests_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return report,
        Err(error) => {
            report.invalid.push(InvalidContest {
                id: "<contests>".to_string(),
                path: contests_dir.to_path_buf(),
                reason: format!("比赛目录不可读: {error}"),
            });
            return report;
        }
    };

    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                report.invalid.push(InvalidContest {
                    id: "<entry>".to_string(),
                    path: contests_dir.to_path_buf(),
                    reason: format!("读取比赛目录项失败: {error}"),
                });
                continue;
            }
        };
        let path = entry.path();
        let is_dir = entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false);
        if !is_dir {
            continue;
        }

        let id = entry.file_name().to_string_lossy().into_owned();
        if !is_valid_contest_id(&id) {
            report.invalid.push(InvalidContest {
                id: id.clone(),
                path,
                reason: "比赛 ID 非法（只允许中英文字母、数字与 ._-，且首字符为字母或数字）"
                    .to_string(),
            });
            continue;
        }

        match load_contest_package(&path, &id) {
            Ok(package) => report.valid.push(Arc::new(package)),
            Err(error) => report.invalid.push(InvalidContest {
                id,
                path,
                reason: error.to_string(),
            }),
        }
    }

    report.valid.sort_by(|a, b| a.id.cmp(&b.id));
    report.invalid.sort_by(|a, b| a.id.cmp(&b.id));
    report
}

pub fn load_contest_package(dir: &Path, id: &str) -> Result<ContestPackage, PackageError> {
    let dir = fs::canonicalize(dir)
        .map_err(|error| PackageError::new(dir, format!("比赛目录不可读: {error}")))?;
    let draft = load_contest_draft(&dir)?;
    let mut problems = Vec::with_capacity(draft.problems.len());
    for problem in draft.problems {
        problems.push(validate_problem(problem)?);
    }

    let roster = Roster::load(&dir.join("students.csv"))?;

    Ok(ContestPackage {
        id: id.to_string(),
        dir,
        title: draft.title,
        duration_minutes: draft.duration_minutes,
        leaderboard_visible: draft.leaderboard_visible,
        problems,
        roster,
    })
}

pub struct ContestRuntime {
    pub package: Arc<ContestPackage>,
    pub store: Arc<ContestStore>,
}

struct RegistryInner {
    runtimes: BTreeMap<String, Arc<ContestRuntime>>,
    invalid: Vec<InvalidContest>,
}

pub struct ContestRegistry {
    inner: RwLock<RegistryInner>,
}

impl ContestRegistry {
    pub fn load(report: ScanReport, data_root: &Path) -> Self {
        let mut runtimes = BTreeMap::new();
        let mut invalid = report.invalid;

        for package in report.valid {
            let data_dir = data_root.join(&package.id);
            match ContestStore::open(&data_dir, &package.id, package.leaderboard_visible) {
                Ok(store) => {
                    runtimes.insert(
                        package.id.clone(),
                        Arc::new(ContestRuntime {
                            package,
                            store: Arc::new(store),
                        }),
                    );
                }
                Err(error) => invalid.push(InvalidContest {
                    id: package.id.clone(),
                    path: data_dir,
                    reason: format!("运行数据库打开失败: {error}"),
                }),
            }
        }

        invalid.sort_by(|a, b| a.id.cmp(&b.id));
        Self {
            inner: RwLock::new(RegistryInner { runtimes, invalid }),
        }
    }

    pub fn empty() -> Self {
        Self {
            inner: RwLock::new(RegistryInner {
                runtimes: BTreeMap::new(),
                invalid: Vec::new(),
            }),
        }
    }

    pub fn get(&self, id: &str) -> Option<Arc<ContestRuntime>> {
        self.read().runtimes.get(id).cloned()
    }

    pub fn list(&self) -> Vec<Arc<ContestRuntime>> {
        self.read().runtimes.values().cloned().collect()
    }

    pub fn invalid(&self) -> Vec<InvalidContest> {
        self.read().invalid.clone()
    }

    pub fn valid_count(&self) -> usize {
        self.read().runtimes.len()
    }

    pub fn remove_runtime(&self, id: &str) -> Option<Arc<ContestRuntime>> {
        self.write().runtimes.remove(id)
    }

    pub fn insert_runtime(&self, runtime: Arc<ContestRuntime>) {
        self.write()
            .runtimes
            .insert(runtime.package.id.clone(), runtime);
    }

    fn read(&self) -> RwLockReadGuard<'_, RegistryInner> {
        self.inner
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, RegistryInner> {
        self.inner
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl Default for ContestRegistry {
    fn default() -> Self {
        Self::empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    fn make_problem(contest: &Path, key: &str) {
        write(
            &contest.join(format!("problems/{key}/problem.json")),
            &format!(
                r#"{{"title":"{key} 题","timeLimitMs":1000,"memoryLimitMb":256,"fullScore":100}}"#
            ),
        );
        write(
            &contest.join(format!("problems/{key}/statement.md")),
            "题面",
        );
        write(
            &contest.join(format!("problems/{key}/testdata/1.in")),
            "1 2",
        );
        write(&contest.join(format!("problems/{key}/testdata/1.out")), "3");
    }

    fn make_contest(root: &Path, id: &str) -> PathBuf {
        let contest = root.join(id);
        write(
            &contest.join("contest.json"),
            r#"{"title":"测试赛","durationMinutes":60,"leaderboardVisible":false}"#,
        );
        write(
            &contest.join("students.csv"),
            "name,student_id,password,school,grade\n张三,2026001,pw,第一中学,七年级\n",
        );
        make_problem(&contest, "A");
        make_problem(&contest, "B");
        contest
    }

    #[test]
    fn scans_valid_contests_in_id_order_with_problem_order() {
        let dir = tempfile::tempdir().unwrap();
        make_contest(dir.path(), "z-contest");
        make_contest(dir.path(), "a-contest");
        write(&dir.path().join("ignored.txt"), "x");

        let report = scan_contests(dir.path());
        assert!(report.invalid.is_empty());
        assert_eq!(report.valid.len(), 2);
        assert_eq!(report.valid[0].id, "a-contest");
        assert_eq!(report.valid[1].id, "z-contest");
        assert_eq!(
            report.valid[0]
                .problems
                .iter()
                .map(|problem| problem.key.as_str())
                .collect::<Vec<_>>(),
            vec!["A", "B"]
        );
        assert_eq!(report.valid[0].roster.students().len(), 1);
        assert!(report.valid[0].find_problem("A").is_some());
        assert!(report.valid[0].find_problem("C").is_none());
    }

    #[test]
    fn invalid_directory_name_goes_to_invalid_list() {
        let dir = tempfile::tempdir().unwrap();
        let contest = make_contest(dir.path(), "bad id");
        let report = scan_contests(dir.path());
        assert!(report.valid.is_empty());
        assert_eq!(report.invalid.len(), 1);
        assert_eq!(report.invalid[0].path, contest);
        assert!(report.invalid[0].reason.contains("ID 非法"));
    }

    #[test]
    fn missing_contest_json_is_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let contest = dir.path().join("c");
        fs::create_dir_all(&contest).unwrap();
        let report = scan_contests(dir.path());
        assert!(report.valid.is_empty());
        assert_eq!(report.invalid.len(), 1);
        assert!(report.invalid[0].reason.contains("contest.json"));
    }

    #[test]
    fn bad_json_and_bad_roster_are_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let contest = dir.path().join("c");
        write(&contest.join("contest.json"), "{bad json");
        let report = scan_contests(dir.path());
        assert_eq!(report.invalid.len(), 1);
        assert!(report.invalid[0].reason.contains("JSON"));

        write(
            &contest.join("contest.json"),
            r#"{"title":"t","durationMinutes":60}"#,
        );
        make_problem(&contest, "A");
        write(&contest.join("students.csv"), "name\n张三\n");
        let report = scan_contests(dir.path());
        assert_eq!(report.invalid.len(), 1);
        assert!(report.invalid[0].reason.contains("student_id"));
    }

    #[test]
    fn invalid_problem_dir_name_and_missing_output_are_invalid() {
        let base = tempfile::tempdir().unwrap();
        let contests = base.path().join("contests");
        let contest = contests.join("c");
        write(
            &contest.join("contest.json"),
            r#"{"title":"t","durationMinutes":60}"#,
        );
        write(
            &contest.join("students.csv"),
            "name,student_id,password\n张三,1,p\n",
        );
        fs::create_dir_all(contest.join("problems/bad key")).unwrap();
        let report = scan_contests(&contests);
        assert_eq!(report.invalid.len(), 1);
        assert!(report.invalid[0].reason.contains("题号"));

        fs::remove_dir_all(contest.join("problems/bad key")).unwrap();
        write(
            &contest.join("problems/A/problem.json"),
            r#"{"title":"A","timeLimitMs":1000,"memoryLimitMb":256,"fullScore":100}"#,
        );
        write(&contest.join("problems/A/statement.md"), "s");
        write(&contest.join("problems/A/testdata/1.in"), "in");
        let report = scan_contests(&contests);
        assert_eq!(report.invalid.len(), 1);
        assert!(report.invalid[0].reason.contains("1.out"));
    }

    #[test]
    fn one_bad_problem_makes_whole_contest_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let contest = make_contest(dir.path(), "c");
        fs::remove_file(contest.join("problems/A/statement.md")).unwrap();
        let report = scan_contests(dir.path());
        assert!(report.valid.is_empty());
        assert_eq!(report.invalid.len(), 1);
        assert!(report.invalid[0].reason.contains("statement.md"));
    }

    #[test]
    fn missing_contests_directory_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let report = scan_contests(&dir.path().join("missing"));
        assert!(report.valid.is_empty());
        assert!(report.invalid.is_empty());
    }

    #[test]
    fn registry_opens_store_for_valid_contest() {
        let dir = tempfile::tempdir().unwrap();
        make_contest(dir.path(), "c");
        let data = dir.path().join("data");
        let report = scan_contests(dir.path());
        let registry = ContestRegistry::load(report, &data);

        assert_eq!(registry.valid_count(), 1);
        assert!(registry.invalid().is_empty());
        let runtime = registry.get("c").unwrap();
        assert_eq!(runtime.package.id, "c");
        assert!(!runtime.store.state().unwrap().leaderboard_visible);
        assert_eq!(registry.list().len(), 1);
        assert!(registry.get("missing").is_none());
        assert!(data.join("c").join("contest.db").is_file());
    }

    #[test]
    fn registry_keeps_scan_invalid_entries() {
        let dir = tempfile::tempdir().unwrap();
        make_contest(dir.path(), "bad id");
        let registry = ContestRegistry::load(scan_contests(dir.path()), &dir.path().join("data"));
        assert_eq!(registry.valid_count(), 0);
        assert_eq!(registry.invalid().len(), 1);
    }

    #[test]
    fn empty_registry_has_no_entries() {
        let registry = ContestRegistry::empty();
        assert_eq!(registry.valid_count(), 0);
        assert!(registry.list().is_empty());
        assert!(registry.invalid().is_empty());
    }
}
