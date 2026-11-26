use std::cmp::Ordering;
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;

use super::model::{
    Checker, ContestDraft, ContestJson, PackageError, ProblemDraft, ProblemJson, ProblemRef,
    SubtaskKind, TestCase, ValidatedProblem, ValidatedSubtask,
};

/// testlib 打开输入/答案文件时有大小上限，超限会退化成 `_fail`；
/// 在加载阶段就挡掉巨型数据，比评测时报「checker 失败」好排查。
const MAX_TEST_FILE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_CHECKER_BYTES: u64 = 1024 * 1024;

pub fn load_json<T: DeserializeOwned>(path: &Path) -> Result<T, PackageError> {
    let text = fs::read_to_string(path)
        .map_err(|error| PackageError::new(path, format!("读取失败: {error}")))?;
    serde_json::from_str(&text).map_err(|error| {
        let mut reason = format!("JSON 解析失败: {error}");
        let text = error.to_string();
        if text.contains("missing field") {
            reason.push_str("\n该字段必须显式填写，没有默认值");
        } else if text.contains("unknown field") {
            reason.push_str("\n字段名拼写有误，请对照 README 的示例");
        }
        PackageError::new(path, reason)
    })
}

/// 比赛 ID（目录名）允许中英文字母、数字，以及 `.` `_` `-`；
/// 首字符必须是字母或数字，避免 `.`、`..` 或隐藏目录被当成比赛。
pub fn is_valid_contest_id(id: &str) -> bool {
    let mut chars = id.chars();
    match chars.next() {
        Some(first) if first.is_alphanumeric() => {}
        _ => return false,
    }
    chars.all(|ch| ch.is_alphanumeric() || matches!(ch, '.' | '_' | '-'))
}

pub fn is_valid_problem_key(key: &str) -> bool {
    (1..=16).contains(&key.len()) && key.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let a = a.as_bytes();
    let b = b.as_bytes();
    let mut i = 0;
    let mut j = 0;

    while i < a.len() && j < b.len() {
        if a[i].is_ascii_digit() && b[j].is_ascii_digit() {
            let a_end = digit_run_end(a, i);
            let b_end = digit_run_end(b, j);
            let a_run = std::str::from_utf8(&a[i..a_end]).unwrap_or("");
            let b_run = std::str::from_utf8(&b[j..b_end]).unwrap_or("");
            let a_number = a_run.trim_start_matches('0');
            let b_number = b_run.trim_start_matches('0');
            let ordering = a_number
                .len()
                .cmp(&b_number.len())
                .then_with(|| a_number.cmp(b_number))
                .then_with(|| a_run.len().cmp(&b_run.len()));
            if ordering != Ordering::Equal {
                return ordering;
            }
            i = a_end;
            j = b_end;
        } else {
            let ordering = a[i].cmp(&b[j]);
            if ordering != Ordering::Equal {
                return ordering;
            }
            i += 1;
            j += 1;
        }
    }

    (a.len() - i).cmp(&(b.len() - j))
}

fn digit_run_end(bytes: &[u8], mut index: usize) -> usize {
    while index < bytes.len() && bytes[index].is_ascii_digit() {
        index += 1;
    }
    index
}

pub fn discover_test_cases(problem_dir: &Path) -> Result<Vec<TestCase>, PackageError> {
    let testdata = problem_dir.join("testdata");
    let entries = fs::read_dir(&testdata)
        .map_err(|error| PackageError::new(&testdata, format!("测试点目录不可读: {error}")))?;

    let mut found: BTreeMap<String, (PathBuf, PathBuf)> = BTreeMap::new();
    for entry in entries {
        let entry = entry
            .map_err(|error| PackageError::new(&testdata, format!("读取测试点失败: {error}")))?;
        let path = entry.path();
        if !entry
            .file_type()
            .map_err(|error| PackageError::new(&path, format!("读取文件类型失败: {error}")))?
            .is_file()
        {
            continue;
        }
        let is_input = path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| extension.eq_ignore_ascii_case("in"))
            .unwrap_or(false);
        if !is_input {
            continue;
        }

        let name = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .ok_or_else(|| PackageError::new(&path, "测试点文件名非法"))?;
        let out_path = testdata.join(format!("{name}.out"));
        if !out_path.is_file() {
            return Err(PackageError::new(
                &out_path,
                format!("测试点 {name} 缺少对应的 .out 文件"),
            ));
        }
        if found
            .insert(name.clone(), (path.clone(), out_path.clone()))
            .is_some()
        {
            return Err(PackageError::new(&path, format!("测试点重名: {name}")));
        }

        for file in [&path, &out_path] {
            let size = fs::metadata(file)
                .map_err(|error| PackageError::new(file, format!("读取大小失败: {error}")))?
                .len();
            if size > MAX_TEST_FILE_BYTES {
                return Err(PackageError::new(
                    file,
                    format!("测试点文件超过 {} MiB", MAX_TEST_FILE_BYTES / 1024 / 1024),
                ));
            }
        }
    }

    if found.is_empty() {
        return Err(PackageError::new(&testdata, "至少需要一个 .in/.out 测试点"));
    }

    let mut test_cases = found
        .into_iter()
        .map(|(name, (in_path, out_path))| TestCase {
            name,
            in_path,
            out_path,
        })
        .collect::<Vec<_>>();
    test_cases.sort_by(|a, b| natural_cmp(&a.name, &b.name));
    Ok(test_cases)
}

/// 强制扫描 `problems/` 下的一级子目录，目录名即题号。
/// 这里先校验目录名本身，随后 `load_contest_draft` 会逐题加载并校验题目内容。
fn discover_problem_refs(root: &Path) -> Result<Vec<ProblemRef>, PackageError> {
    let problems_dir = root.join("problems");
    let entries = fs::read_dir(&problems_dir).map_err(|error| {
        PackageError::new(&problems_dir, format!("problems 目录不可读: {error}"))
    })?;

    let mut refs = Vec::new();
    let mut seen_folded_keys = HashSet::new();
    for entry in entries {
        let entry = entry.map_err(|error| {
            PackageError::new(&problems_dir, format!("扫描 problems 目录失败: {error}"))
        })?;
        // 目录里可能有 README 之类的文件，只认一级子目录。
        if !entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
            continue;
        }
        let key = entry.file_name().to_string_lossy().into_owned();
        if !is_valid_problem_key(&key) {
            return Err(PackageError::new(
                entry.path(),
                format!("题号（目录名）非法: {key}（只允许 1～16 位 ASCII 字母或数字）"),
            ));
        }
        // 题号会出现在 checker 产物与提交目录里，Windows 不区分大小写。
        if !seen_folded_keys.insert(key.to_lowercase()) {
            return Err(PackageError::new(
                entry.path(),
                format!("题号只有大小写不同: {key}（文件系统不区分大小写）"),
            ));
        }
        let dir = fs::canonicalize(entry.path())
            .map_err(|error| PackageError::new(entry.path(), format!("题目目录不可读: {error}")))?;
        if !dir.starts_with(root) {
            return Err(PackageError::new(entry.path(), "题目目录逃逸出比赛目录"));
        }
        refs.push(ProblemRef { key, dir });
    }
    refs.sort_by(|left, right| natural_cmp(&left.key, &right.key));
    if refs.is_empty() {
        return Err(PackageError::new(
            &problems_dir,
            "至少需要一个题目（problems 目录为空）",
        ));
    }
    Ok(refs)
}

pub fn validate_contest_header(
    contest_dir: &Path,
    json: &ContestJson,
) -> Result<Vec<ProblemRef>, PackageError> {
    let root = fs::canonicalize(contest_dir).map_err(|error| {
        PackageError::new(contest_dir, format!("比赛目录不存在或不可读: {error}"))
    })?;

    let title = json.title.trim();
    if title.is_empty() {
        return Err(PackageError::new(
            root.join("contest.json"),
            "title 不能为空",
        ));
    }
    if title.chars().count() > 120 {
        return Err(PackageError::new(
            root.join("contest.json"),
            "title 超过 120 字符",
        ));
    }
    if !(1..=600).contains(&json.duration_minutes) {
        return Err(PackageError::new(
            root.join("contest.json"),
            "durationMinutes 必须在 1～600 之间",
        ));
    }
    discover_problem_refs(&root)
}

pub fn load_problem_draft(
    contest_root: &Path,
    reference: &ProblemRef,
    json: &ProblemJson,
) -> Result<ProblemDraft, PackageError> {
    let root = fs::canonicalize(contest_root).map_err(|error| {
        PackageError::new(contest_root, format!("比赛目录不存在或不可读: {error}"))
    })?;
    let dir = fs::canonicalize(&reference.dir)
        .map_err(|error| PackageError::new(&reference.dir, format!("题目目录不可读: {error}")))?;
    if !dir.starts_with(&root) {
        return Err(PackageError::new(&reference.dir, "题目目录逃逸出比赛目录"));
    }
    let problem_json = dir.join("problem.json");

    let title = json.title.trim();
    if title.is_empty() {
        return Err(PackageError::new(&problem_json, "title 不能为空"));
    }
    if title.chars().count() > 200 {
        return Err(PackageError::new(&problem_json, "title 超过 200 字符"));
    }
    if !(100..=60_000).contains(&json.time_limit_ms) {
        return Err(PackageError::new(
            &problem_json,
            "timeLimitMs 必须在 100～60000 之间",
        ));
    }
    if !(16..=2048).contains(&json.memory_limit_mb) {
        return Err(PackageError::new(
            &problem_json,
            "memoryLimitMb 必须在 16～2048 之间",
        ));
    }

    let statement_path = dir.join("statement.md");
    if !statement_path.is_file() {
        return Err(PackageError::new(&statement_path, "缺少 statement.md"));
    }
    let test_cases = discover_test_cases(&dir)?;
    let checker = resolve_checker(&dir, json)?;

    Ok(ProblemDraft {
        key: reference.key.clone(),
        title: title.to_string(),
        time_limit_ms: json.time_limit_ms,
        memory_limit_mb: json.memory_limit_mb,
        full_score: json.full_score,
        subtasks: json.subtasks.clone(),
        root: dir,
        statement_path,
        test_cases,
        checker,
    })
}

/// 固定文件名 `checker.cpp`；`enabled = false` 时即使文件存在也不启用。
fn resolve_checker(dir: &Path, json: &ProblemJson) -> Result<Option<Checker>, PackageError> {
    let Some(config) = &json.checker else {
        return Ok(None);
    };
    if !config.enabled {
        return Ok(None);
    }
    let problem_json = dir.join("problem.json");
    if !(100..=60_000).contains(&config.time_limit_ms) {
        return Err(PackageError::new(
            &problem_json,
            "checker.timeLimitMs 必须在 100～60000 之间",
        ));
    }
    if !(16..=2048).contains(&config.memory_limit_mb) {
        return Err(PackageError::new(
            &problem_json,
            "checker.memoryLimitMb 必须在 16～2048 之间",
        ));
    }

    let source = dir.join("checker.cpp");
    if !source.is_file() {
        return Err(PackageError::new(
            &source,
            "已启用 checker，但题目目录缺少 checker.cpp（不想启用请把 enabled 改成 false）",
        ));
    }
    let size = fs::metadata(&source)
        .map_err(|error| PackageError::new(&source, format!("读取大小失败: {error}")))?
        .len();
    if size > MAX_CHECKER_BYTES {
        return Err(PackageError::new(&source, "checker.cpp 超过 1 MiB"));
    }
    if size == 0 {
        return Err(PackageError::new(&source, "checker.cpp 是空文件"));
    }

    Ok(Some(Checker {
        source,
        time_limit_ms: config.time_limit_ms,
        memory_limit_mb: config.memory_limit_mb,
    }))
}

pub fn load_contest_draft(contest_dir: &Path) -> Result<ContestDraft, PackageError> {
    let root = fs::canonicalize(contest_dir).map_err(|error| {
        PackageError::new(contest_dir, format!("比赛目录不存在或不可读: {error}"))
    })?;
    let contest_json_path = root.join("contest.json");
    let json: ContestJson = load_json(&contest_json_path)?;
    let references = validate_contest_header(&root, &json)?;

    let mut problems = Vec::with_capacity(references.len());
    for reference in references {
        let problem_json_path = reference.dir.join("problem.json");
        let problem_json: ProblemJson = load_json(&problem_json_path)?;
        problems.push(load_problem_draft(&root, &reference, &problem_json)?);
    }

    Ok(ContestDraft {
        title: json.title.trim().to_string(),
        duration_minutes: json.duration_minutes,
        leaderboard_visible: json.leaderboard_visible,
        problems,
    })
}

pub fn validate_problem(draft: ProblemDraft) -> Result<ValidatedProblem, PackageError> {
    let problem_json = draft.root.join("problem.json");
    let test_names = draft
        .test_cases
        .iter()
        .map(|case| case.name.clone())
        .collect::<Vec<_>>();
    let known = test_names
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();

    let (subtasks, full_score) = match draft.subtasks {
        None => {
            let full_score = draft
                .full_score
                .ok_or_else(|| PackageError::new(&problem_json, "缺少 fullScore"))?;
            if !(1..=1000).contains(&full_score) {
                return Err(PackageError::new(
                    &problem_json,
                    "fullScore 必须在 1～1000 之间",
                ));
            }
            let subtask = ValidatedSubtask {
                name: "子任务 1".to_string(),
                score: full_score,
                kind: SubtaskKind::Sum,
                cases: test_names.clone(),
            };
            (vec![subtask], full_score)
        }
        Some(raw_subtasks) => {
            if raw_subtasks.is_empty() {
                return Err(PackageError::new(&problem_json, "subtasks 不能为空数组"));
            }

            let mut seen = HashSet::new();
            let mut validated = Vec::with_capacity(raw_subtasks.len());
            let mut total = 0i32;

            for (index, subtask) in raw_subtasks.into_iter().enumerate() {
                if !(1..=1000).contains(&subtask.score) {
                    return Err(PackageError::new(
                        &problem_json,
                        format!("子任务 {} 的 score 必须在 1～1000 之间", index + 1),
                    ));
                }
                total += subtask.score;
                if total > 1000 {
                    return Err(PackageError::new(&problem_json, "子任务分值总和超过 1000"));
                }

                let name = subtask
                    .name
                    .map(|name| name.trim().to_string())
                    .filter(|name| !name.is_empty())
                    .unwrap_or_else(|| format!("子任务 {}", index + 1));
                if name.chars().count() > 32 {
                    return Err(PackageError::new(
                        &problem_json,
                        format!("子任务名超过 32 字符: {name}"),
                    ));
                }
                if subtask.cases.is_empty() {
                    return Err(PackageError::new(
                        &problem_json,
                        format!("子任务 {name} 的 cases 不能为空"),
                    ));
                }
                for case in &subtask.cases {
                    if !known.contains(case.as_str()) {
                        return Err(PackageError::new(
                            &problem_json,
                            format!("子任务 {name} 引用了未知测试点: {case}"),
                        ));
                    }
                    if !seen.insert(case.clone()) {
                        return Err(PackageError::new(
                            &problem_json,
                            format!("测试点在多个子任务中重复出现: {case}"),
                        ));
                    }
                }

                validated.push(ValidatedSubtask {
                    name,
                    score: subtask.score,
                    kind: subtask.kind,
                    cases: subtask.cases,
                });
            }

            if seen.len() != known.len() {
                let missing = test_names
                    .iter()
                    .filter(|name| !seen.contains(*name))
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(PackageError::new(
                    &problem_json,
                    format!("有测试点未被任何子任务引用: {missing}"),
                ));
            }

            if let Some(explicit) = draft.full_score {
                if explicit != total {
                    return Err(PackageError::new(
                        &problem_json,
                        format!("fullScore={explicit} 与子任务分值和 {total} 不一致"),
                    ));
                }
            }

            (validated, total)
        }
    };

    Ok(ValidatedProblem {
        key: draft.key,
        title: draft.title,
        time_limit_ms: draft.time_limit_ms,
        memory_limit_mb: draft.memory_limit_mb,
        full_score,
        root: draft.root,
        statement_path: draft.statement_path,
        test_cases: draft.test_cases,
        subtasks,
        checker: draft.checker,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contest::model::{ProblemJson, SubtaskJson};
    use std::fs;

    fn write(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    fn contest_json(title: &str, duration: u32) -> ContestJson {
        ContestJson {
            title: title.to_string(),
            duration_minutes: duration,
            leaderboard_visible: false,
        }
    }

    fn problem_json(title: &str, time: u32, memory: u32) -> ProblemJson {
        ProblemJson {
            title: title.to_string(),
            time_limit_ms: time,
            memory_limit_mb: memory,
            full_score: Some(100),
            subtasks: None,
            checker: None,
        }
    }

    #[test]
    fn contest_id_accepts_chinese_and_safe_ascii() {
        for id in ["demo", "2026秋季赛", "中文-比赛_1", "A.1"] {
            assert!(is_valid_contest_id(id), "{id} 应当合法");
        }
    }

    #[test]
    fn contest_id_rejects_path_and_separator_characters() {
        for id in [
            "", ".hidden", "../demo", "a/b", "a\\b", "a~b", "a b", "赛/1",
        ] {
            assert!(!is_valid_contest_id(id), "{id} 应当非法");
        }
    }

    #[test]
    fn natural_sort_orders_numbers_by_value() {
        let mut names = vec!["10", "2", "1", "a10", "a2"];
        names.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(names, vec!["1", "2", "10", "a2", "a10"]);
    }

    #[test]
    fn natural_sort_is_total_for_leading_zeroes() {
        assert_eq!(natural_cmp("01", "1"), Ordering::Greater);
        assert_eq!(natural_cmp("2", "10"), Ordering::Less);
        assert_eq!(natural_cmp("a2", "a10"), Ordering::Less);
    }

    #[test]
    fn test_cases_are_paired_and_sorted() {
        let dir = tempfile::tempdir().unwrap();
        let testdata = dir.path().join("testdata");
        for name in ["10", "2", "1"] {
            write(&testdata.join(format!("{name}.in")), "in");
            write(&testdata.join(format!("{name}.out")), "out");
        }
        write(&testdata.join("readme.txt"), "ignored");

        let cases = discover_test_cases(dir.path()).unwrap();
        assert_eq!(
            cases
                .iter()
                .map(|case| case.name.as_str())
                .collect::<Vec<_>>(),
            vec!["1", "2", "10"]
        );
    }

    #[test]
    fn test_case_without_output_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let testdata = dir.path().join("testdata");
        write(&testdata.join("3.in"), "in");
        let error = discover_test_cases(dir.path()).unwrap_err();
        assert!(error.to_string().contains("3.out"));
    }

    #[test]
    fn empty_testdata_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("testdata")).unwrap();
        let error = discover_test_cases(dir.path()).unwrap_err();
        assert!(error.to_string().contains("至少需要一个"));
    }

    #[test]
    fn contest_header_ignores_leftover_problems_field() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("problems/A")).unwrap();
        fs::create_dir_all(dir.path().join("problems/B")).unwrap();
        // 项目从未发包，旧包里可能残留 problems 字段；serde 静默跳过，仍按目录扫描。
        let json: ContestJson = serde_json::from_str(
            r#"{
                "title": "测试赛",
                "durationMinutes": 60,
                "leaderboardVisible": true,
                "problems": [{"key": "B", "dir": "problems/B"}]
            }"#,
        )
        .unwrap();

        let refs = validate_contest_header(dir.path(), &json).unwrap();
        assert_eq!(
            refs.iter()
                .map(|item| item.key.as_str())
                .collect::<Vec<_>>(),
            vec!["A", "B"]
        );
        assert!(refs[0]
            .dir
            .starts_with(fs::canonicalize(dir.path()).unwrap()));
    }

    #[test]
    fn contest_header_scans_problems_directory() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("problems/B")).unwrap();
        fs::create_dir_all(dir.path().join("problems/A")).unwrap();
        fs::write(dir.path().join("problems/README.md"), "not a problem").unwrap();

        let refs = validate_contest_header(dir.path(), &contest_json("t", 60)).unwrap();
        assert_eq!(
            refs.iter()
                .map(|item| item.key.as_str())
                .collect::<Vec<_>>(),
            vec!["A", "B"]
        );
        assert!(refs[0].dir.ends_with("problems/A"));
    }

    #[test]
    fn contest_header_requires_problems_directory() {
        let dir = tempfile::tempdir().unwrap();
        let error = validate_contest_header(dir.path(), &contest_json("t", 60)).unwrap_err();
        assert!(error.to_string().contains("problems 目录不可读"));
    }

    #[test]
    fn contest_header_rejects_bad_fields_and_dir_names() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("problems/A")).unwrap();

        for case in [
            contest_json("", 60),
            contest_json(&"x".repeat(121), 60),
            contest_json("t", 0),
            contest_json("t", 601),
        ] {
            assert!(validate_contest_header(dir.path(), &case).is_err());
        }

        let invalid = tempfile::tempdir().unwrap();
        fs::create_dir_all(invalid.path().join("problems/bad key")).unwrap();
        let error = validate_contest_header(invalid.path(), &contest_json("t", 60)).unwrap_err();
        assert!(error.to_string().contains("题号"));

        let empty = tempfile::tempdir().unwrap();
        fs::create_dir_all(empty.path().join("problems")).unwrap();
        let error = validate_contest_header(empty.path(), &contest_json("t", 60)).unwrap_err();
        assert!(error.to_string().contains("至少需要一个题目"));
    }

    #[test]
    fn problem_draft_rejects_bad_fields() {
        let dir = tempfile::tempdir().unwrap();
        let problem = dir.path().join("problems/A");
        write(&problem.join("statement.md"), "statement");
        write(&problem.join("testdata/1.in"), "in");
        write(&problem.join("testdata/1.out"), "out");

        let reference = ProblemRef {
            key: "A".to_string(),
            dir: fs::canonicalize(&problem).unwrap(),
        };

        let bad = [
            problem_json("", 1000, 256),
            problem_json(&"x".repeat(201), 1000, 256),
            problem_json("t", 99, 256),
            problem_json("t", 60_001, 256),
            problem_json("t", 1000, 15),
            problem_json("t", 1000, 2049),
        ];

        for json in bad {
            assert!(load_problem_draft(dir.path(), &reference, &json).is_err());
        }

        fs::remove_file(problem.join("statement.md")).unwrap();
        assert!(load_problem_draft(dir.path(), &reference, &problem_json("t", 1000, 256)).is_err());
    }

    #[test]
    fn contest_draft_loads_problem_order() {
        let dir = tempfile::tempdir().unwrap();
        let contest = dir.path().join("c");
        write(
            &contest.join("contest.json"),
            r#"{"title":"T","durationMinutes":60}"#,
        );
        for key in ["A", "B"] {
            write(
                &contest.join(format!("problems/{key}/problem.json")),
                &format!(
                    r#"{{"title":"{key}","timeLimitMs":1000,"memoryLimitMb":256,"fullScore":100}}"#
                ),
            );
            write(&contest.join(format!("problems/{key}/statement.md")), "s");
            write(&contest.join(format!("problems/{key}/testdata/1.in")), "in");
            write(
                &contest.join(format!("problems/{key}/testdata/1.out")),
                "out",
            );
        }

        let draft = load_contest_draft(&contest).unwrap();
        assert_eq!(draft.problems[0].key, "A");
        assert_eq!(draft.problems[1].key, "B");
        assert_eq!(draft.problems[0].test_cases.len(), 1);
    }

    fn checker_problem(
        dir: &Path,
        json: &str,
        checker_source: Option<&str>,
    ) -> Result<ProblemDraft, PackageError> {
        let problem = dir.join("problems/A");
        write(&problem.join("statement.md"), "s");
        write(&problem.join("testdata/1.in"), "in");
        write(&problem.join("testdata/1.out"), "out");
        if let Some(source) = checker_source {
            write(&problem.join("checker.cpp"), source);
        }
        write(&problem.join("problem.json"), json);
        let reference = ProblemRef {
            key: "A".to_string(),
            dir: fs::canonicalize(&problem).unwrap(),
        };
        let json: ProblemJson = load_json(&problem.join("problem.json")).unwrap();
        load_problem_draft(dir, &reference, &json)
    }

    fn checker_json(enabled: bool, time_limit_ms: u32, memory_limit_mb: u32) -> String {
        format!(
            r#"{{"title":"A","timeLimitMs":1000,"memoryLimitMb":256,"fullScore":100,
                "checker":{{"enabled":{enabled},"timeLimitMs":{time_limit_ms},"memoryLimitMb":{memory_limit_mb}}}}}"#
        )
    }

    fn checker_json_of(checker: &str) -> String {
        format!(
            r#"{{"title":"A","timeLimitMs":1000,"memoryLimitMb":256,"fullScore":100,"checker":{checker}}}"#
        )
    }

    #[test]
    fn checker_config_has_no_defaults() {
        // 三项都必填：缺任何一项都要在加载阶段报出来，而不是悄悄按默认值跑。
        for (checker, missing) in [
            (r#"{"timeLimitMs":5000,"memoryLimitMb":512}"#, "enabled"),
            (r#"{"enabled":true,"memoryLimitMb":512}"#, "timeLimitMs"),
            (r#"{"enabled":true,"timeLimitMs":5000}"#, "memoryLimitMb"),
            ("{}", "enabled"),
        ] {
            let error = serde_json::from_str::<ProblemJson>(&checker_json_of(checker)).unwrap_err();
            assert!(
                error.to_string().contains(missing),
                "{checker} 应当报缺少 {missing}，实际: {error}"
            );
        }

        // 拼错字段名同样报错（deny_unknown_fields），不会被当成「少写一项、用默认值」。
        let typo = r#"{"enabled":true,"timeLimitMs":5000,"memoryLimitMB":512}"#;
        assert!(serde_json::from_str::<ProblemJson>(&checker_json_of(typo)).is_err());
    }

    #[test]
    fn checker_is_off_unless_enabled() {
        let dir = tempfile::tempdir().unwrap();
        // 没有 checker 段：即使目录里有 checker.cpp 也忽略（完全相信教师）。
        let base = r#"{"title":"A","timeLimitMs":1000,"memoryLimitMb":256,"fullScore":100}"#;
        let draft = checker_problem(dir.path(), base, Some("int main(){}")).unwrap();
        assert!(draft.checker.is_none());

        // 声明了但 enabled=false：有文件也不用。
        let draft = checker_problem(
            dir.path(),
            &checker_json(false, 5000, 512),
            Some("int main(){}"),
        )
        .unwrap();
        assert!(draft.checker.is_none());
    }

    #[test]
    fn enabled_checker_needs_source_and_keeps_configured_limits() {
        let dir = tempfile::tempdir().unwrap();
        let draft = checker_problem(
            dir.path(),
            &checker_json(true, 8000, 256),
            Some("int main(){}"),
        )
        .unwrap();
        let checker = draft.checker.expect("应当启用 checker");
        assert_eq!(checker.time_limit_ms, 8000);
        assert_eq!(checker.memory_limit_mb, 256);
        assert!(checker.source.ends_with("checker.cpp"));

        let dir = tempfile::tempdir().unwrap();
        let error = checker_problem(dir.path(), &checker_json(true, 5000, 512), None).unwrap_err();
        assert!(error.reason.contains("checker.cpp"));
    }

    #[test]
    fn checker_limits_are_validated() {
        let dir = tempfile::tempdir().unwrap();
        let error = checker_problem(
            dir.path(),
            &checker_json(true, 10, 512),
            Some("int main(){}"),
        )
        .unwrap_err();
        assert!(error.reason.contains("timeLimitMs"));

        let dir = tempfile::tempdir().unwrap();
        let error = checker_problem(
            dir.path(),
            &checker_json(true, 5000, 9999),
            Some("int main(){}"),
        )
        .unwrap_err();
        assert!(error.reason.contains("memoryLimitMb"));
    }

    #[test]
    fn checker_source_must_not_be_empty_or_huge() {
        let dir = tempfile::tempdir().unwrap();
        let error =
            checker_problem(dir.path(), &checker_json(true, 5000, 512), Some("")).unwrap_err();
        assert!(error.reason.contains("空文件"));

        let dir = tempfile::tempdir().unwrap();
        let huge = "a".repeat(1024 * 1024 + 1);
        let error =
            checker_problem(dir.path(), &checker_json(true, 5000, 512), Some(&huge)).unwrap_err();
        assert!(error.reason.contains("1 MiB"));
    }

    #[test]
    fn problem_with_subtasks_keeps_raw_rules_for_later_validation() {
        let dir = tempfile::tempdir().unwrap();
        let problem = dir.path().join("problems/A");
        write(&problem.join("statement.md"), "s");
        write(&problem.join("testdata/1.in"), "in");
        write(&problem.join("testdata/1.out"), "out");
        let reference = ProblemRef {
            key: "A".to_string(),
            dir: fs::canonicalize(&problem).unwrap(),
        };
        let json = ProblemJson {
            title: "A".to_string(),
            time_limit_ms: 1000,
            memory_limit_mb: 256,
            full_score: None,
            subtasks: Some(vec![SubtaskJson {
                name: None,
                score: 100,
                kind: Default::default(),
                cases: vec!["1".to_string()],
            }]),
            checker: None,
        };
        let draft = load_problem_draft(dir.path(), &reference, &json).unwrap();
        assert!(draft.full_score.is_none());
        assert_eq!(draft.subtasks.unwrap()[0].cases, vec!["1"]);
    }

    fn draft(
        full_score: Option<i32>,
        subtasks: Option<Vec<SubtaskJson>>,
        names: &[&str],
    ) -> ProblemDraft {
        let root = PathBuf::from("C:/test/problem");
        let test_cases = names
            .iter()
            .map(|name| TestCase {
                name: (*name).to_string(),
                in_path: root.join("testdata").join(format!("{name}.in")),
                out_path: root.join("testdata").join(format!("{name}.out")),
            })
            .collect();
        ProblemDraft {
            key: "A".to_string(),
            title: "A".to_string(),
            time_limit_ms: 1000,
            memory_limit_mb: 256,
            full_score,
            subtasks,
            root: root.clone(),
            statement_path: root.join("statement.md"),
            test_cases,
            checker: None,
        }
    }

    fn subtask(score: i32, kind: SubtaskKind, cases: &[&str]) -> SubtaskJson {
        SubtaskJson {
            name: None,
            score,
            kind,
            cases: cases.iter().map(|case| (*case).to_string()).collect(),
        }
    }

    #[test]
    fn implicit_subtask_is_created_when_subtasks_are_missing() {
        let problem = validate_problem(draft(Some(100), None, &["1", "2", "3"])).unwrap();
        assert_eq!(problem.full_score, 100);
        assert_eq!(problem.subtasks.len(), 1);
        assert_eq!(problem.subtasks[0].name, "子任务 1");
        assert_eq!(problem.subtasks[0].kind, SubtaskKind::Sum);
        assert_eq!(problem.subtasks[0].cases, vec!["1", "2", "3"]);
    }

    #[test]
    fn implicit_path_requires_valid_full_score() {
        assert!(validate_problem(draft(None, None, &["1"])).is_err());
        assert!(validate_problem(draft(Some(0), None, &["1"])).is_err());
        assert!(validate_problem(draft(Some(1001), None, &["1"])).is_err());
    }

    #[test]
    fn subtasks_derive_full_score_and_default_names() {
        let problem = validate_problem(draft(
            None,
            Some(vec![
                subtask(30, SubtaskKind::Sum, &["1", "2"]),
                subtask(70, SubtaskKind::Min, &["3"]),
            ]),
            &["1", "2", "3"],
        ))
        .unwrap();

        assert_eq!(problem.full_score, 100);
        assert_eq!(problem.subtasks[0].name, "子任务 1");
        assert_eq!(problem.subtasks[0].kind, SubtaskKind::Sum);
        assert_eq!(problem.subtasks[1].name, "子任务 2");
        assert_eq!(problem.subtasks[1].kind, SubtaskKind::Min);
    }

    #[test]
    fn explicit_full_score_must_match_sum() {
        assert!(validate_problem(draft(
            Some(100),
            Some(vec![subtask(100, SubtaskKind::Max, &["1"])]),
            &["1"],
        ))
        .is_ok());

        let error = validate_problem(draft(
            Some(90),
            Some(vec![subtask(100, SubtaskKind::Max, &["1"])]),
            &["1"],
        ))
        .unwrap_err();
        assert!(error.to_string().contains("不一致"));
    }

    #[test]
    fn subtasks_reject_bad_scores_and_empty_list() {
        let cases = ["1"];
        assert!(validate_problem(draft(None, Some(vec![]), &cases)).is_err());
        assert!(validate_problem(draft(
            None,
            Some(vec![subtask(0, SubtaskKind::Sum, &cases)]),
            &cases
        ))
        .is_err());
        assert!(validate_problem(draft(
            None,
            Some(vec![subtask(1001, SubtaskKind::Sum, &cases)]),
            &cases
        ))
        .is_err());
        assert!(validate_problem(draft(
            None,
            Some(vec![
                subtask(600, SubtaskKind::Sum, &cases),
                subtask(600, SubtaskKind::Sum, &cases),
            ]),
            &cases,
        ))
        .is_err());
    }

    #[test]
    fn subtasks_reject_unknown_duplicate_and_missing_cases() {
        let unknown = validate_problem(draft(
            None,
            Some(vec![subtask(100, SubtaskKind::Sum, &["1", "9"])]),
            &["1"],
        ))
        .unwrap_err();
        assert!(unknown.to_string().contains("未知测试点"));

        let duplicate = validate_problem(draft(
            None,
            Some(vec![
                subtask(50, SubtaskKind::Sum, &["1"]),
                subtask(50, SubtaskKind::Sum, &["1"]),
            ]),
            &["1"],
        ))
        .unwrap_err();
        assert!(duplicate.to_string().contains("重复"));

        let missing = validate_problem(draft(
            None,
            Some(vec![subtask(100, SubtaskKind::Sum, &["1"])]),
            &["1", "2"],
        ))
        .unwrap_err();
        assert!(missing.to_string().contains("未被任何子任务引用"));
    }

    #[test]
    fn subtasks_reject_long_names_and_empty_cases() {
        let mut long_name = subtask(100, SubtaskKind::Sum, &["1"]);
        long_name.name = Some("x".repeat(33));
        assert!(validate_problem(draft(None, Some(vec![long_name]), &["1"])).is_err());

        let empty_cases = subtask(100, SubtaskKind::Sum, &[]);
        assert!(validate_problem(draft(None, Some(vec![empty_cases]), &["1"])).is_err());
    }
}
