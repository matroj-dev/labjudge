use std::collections::HashSet;
use std::fs;
use std::path::Path;

use super::model::PackageError;

/// 口令取学生 ID 的后 6 个字符。
///
/// ID 由老师设定（学号、身份证号都行），学生自己知道，因此不需要
/// 事先发放口令，名单里也不必额外维护一列。ID 不足 6 位时整串都是口令。
const PASSWORD_SUFFIX_LEN: usize = 6;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Student {
    pub student_id: String,
    pub name: String,
    pub school: String,
    pub grade: String,
}

#[derive(Debug, Clone)]
pub struct Roster {
    students: Vec<Student>,
}

pub fn password_for(student_id: &str) -> String {
    let chars: Vec<char> = student_id.chars().collect();
    let start = chars.len().saturating_sub(PASSWORD_SUFFIX_LEN);
    chars[start..].iter().collect()
}

impl Roster {
    pub fn load(path: &Path) -> Result<Self, PackageError> {
        let raw = fs::read_to_string(path)
            .map_err(|error| PackageError::new(path, format!("读取失败: {error}")))?;
        let text = raw.strip_prefix('\u{feff}').unwrap_or(&raw);

        let mut reader = csv::ReaderBuilder::new()
            .trim(csv::Trim::All)
            .flexible(true)
            .from_reader(text.as_bytes());

        let headers = reader
            .headers()
            .map_err(|error| PackageError::new(path, format!("表头解析失败: {error}")))?
            .clone();
        let index = |name: &str| headers.iter().position(|header| header.trim() == name);
        let name_index = index("name").ok_or_else(|| PackageError::new(path, "缺少 name 列"))?;
        let id_index =
            index("student_id").ok_or_else(|| PackageError::new(path, "缺少 student_id 列"))?;
        let school_index = index("school");
        let grade_index = index("grade");

        let mut students = Vec::new();
        let mut seen_ids = HashSet::new();
        for (row, record) in reader.records().enumerate() {
            let record = record.map_err(|error| {
                PackageError::new(path, format!("第 {} 行解析失败: {error}", row + 2))
            })?;
            if record.iter().all(|field| field.trim().is_empty()) {
                continue;
            }

            let name = field(&record, name_index, "name", path, row)?;
            let student_id = field(&record, id_index, "student_id", path, row)?;
            check_length(&name, 64, "name", path, row)?;
            check_length(&student_id, 64, "student_id", path, row)?;

            let school = school_index
                .map(|index| record.get(index).unwrap_or("").trim().to_string())
                .unwrap_or_default();
            let grade = grade_index
                .map(|index| record.get(index).unwrap_or("").trim().to_string())
                .unwrap_or_default();
            check_length(&school, 64, "school", path, row)?;
            check_length(&grade, 64, "grade", path, row)?;

            if !seen_ids.insert(student_id.clone()) {
                return Err(PackageError::new(path, format!("ID 重复: {student_id}")));
            }

            students.push(Student {
                student_id,
                name,
                school,
                grade,
            });
        }

        if students.is_empty() {
            return Err(PackageError::new(path, "名单为空"));
        }

        Ok(Self { students })
    }

    pub fn students(&self) -> &[Student] {
        &self.students
    }

    /// ID 是唯一凭据，口令由 ID 派生，两者都不匹配时统一返回 `None`。
    pub fn authenticate(&self, student_id: &str, password: &str) -> Option<Student> {
        self.students
            .iter()
            .find(|student| {
                student.student_id == student_id && password_for(&student.student_id) == password
            })
            .cloned()
    }
}

fn field(
    record: &csv::StringRecord,
    index: usize,
    field: &str,
    path: &Path,
    row: usize,
) -> Result<String, PackageError> {
    let value = record.get(index).unwrap_or("").trim().to_string();
    if value.is_empty() {
        return Err(PackageError::new(
            path,
            format!("第 {} 行缺少必填字段 {field}", row + 2),
        ));
    }
    Ok(value)
}

fn check_length(
    value: &str,
    max: usize,
    field: &str,
    path: &Path,
    row: usize,
) -> Result<(), PackageError> {
    if value.chars().count() > max {
        return Err(PackageError::new(
            path,
            format!("第 {} 行 {field} 超过 {max} 字符", row + 2),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, content: &str) {
        fs::write(path, content).unwrap();
    }

    fn student(student_id: &str) -> Student {
        Student {
            student_id: student_id.to_string(),
            name: "张三".to_string(),
            school: String::new(),
            grade: String::new(),
        }
    }

    #[test]
    fn parses_bom_quotes_whitespace_and_unknown_columns() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("students.csv");
        write(
            &path,
            "\u{feff}name,student_id,school,grade,extra\n\"张,三\", 2026001 , 第一中学 ,七年级,x\n李四,2026002,,,\n",
        );
        let roster = Roster::load(&path).unwrap();
        assert_eq!(roster.students().len(), 2);
        assert_eq!(roster.students()[0].name, "张,三");
        assert_eq!(roster.students()[0].student_id, "2026001");
        assert_eq!(roster.students()[0].school, "第一中学");
        assert_eq!(roster.students()[1].school, "");
        assert_eq!(roster.students()[1].grade, "");
    }

    #[test]
    fn rejects_missing_headers_and_bad_rows() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("students.csv");

        write(&path, "student_id\n2026001\n");
        assert!(Roster::load(&path)
            .unwrap_err()
            .to_string()
            .contains("name"));

        write(&path, "name\n张三\n");
        assert!(Roster::load(&path)
            .unwrap_err()
            .to_string()
            .contains("student_id"));

        write(&path, "name,student_id\n,2026001\n");
        assert!(Roster::load(&path).is_err());

        write(&path, "name,student_id\n张三,\n");
        assert!(Roster::load(&path).is_err());

        write(&path, "name,student_id\n, \n");
        assert!(Roster::load(&path).is_err());
    }

    #[test]
    fn rejects_long_fields_and_duplicate_student_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("students.csv");

        write(
            &path,
            &format!("name,student_id\n{},2026001\n", "x".repeat(65)),
        );
        assert!(Roster::load(&path).is_err());

        write(
            &path,
            &format!("name,student_id\n张三,{}\n", "1".repeat(65)),
        );
        assert!(Roster::load(&path).is_err());

        write(
            &path,
            "name,student_id,school\n张三,2026001,第一中学\n李四,2026001,第二中学\n",
        );
        let error = Roster::load(&path).unwrap_err().to_string();
        assert!(error.contains("ID 重复"));
    }

    #[test]
    fn rejects_empty_roster() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("students.csv");
        write(&path, "name,student_id\n");
        assert!(Roster::load(&path)
            .unwrap_err()
            .to_string()
            .contains("名单为空"));
    }

    #[test]
    fn password_is_last_six_characters_of_id() {
        assert_eq!(password_for("110101200001011234"), "011234");
        assert_eq!(password_for("2026001"), "026001");
        // 短 ID 整串都是口令，不会 panic。
        assert_eq!(password_for("12345"), "12345");
        assert_eq!(password_for(""), "");
    }

    #[test]
    fn authenticates_with_id_suffix() {
        let roster = Roster {
            students: vec![student("110101200001011234"), student("110101200001015678")],
        };

        assert_eq!(
            roster.authenticate("110101200001011234", "011234"),
            Some(roster.students[0].clone())
        );
        // 拿别人的口令登自己的 ID 也不行。
        assert_eq!(roster.authenticate("110101200001011234", "015678"), None);
        assert_eq!(roster.authenticate("110101200001019999", "011234"), None);
        // 大小写敏感：以身份证号作 ID 时末位可能是 X。
        assert_eq!(roster.authenticate("11010120000101123X", "01123X"), None);
    }
}
