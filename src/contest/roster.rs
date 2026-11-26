use std::collections::HashSet;
use std::fmt;
use std::fs;
use std::path::Path;

use super::model::PackageError;

#[derive(Clone, PartialEq, Eq)]
pub struct Student {
    pub student_id: String,
    pub name: String,
    pub school: String,
    pub grade: String,
    pub(crate) password: String,
}

/// 口令不进入日志。
impl fmt::Debug for Student {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Student")
            .field("student_id", &self.student_id)
            .field("name", &self.name)
            .field("school", &self.school)
            .field("grade", &self.grade)
            .field("password", &"***")
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct Roster {
    students: Vec<Student>,
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
        let password_index =
            index("password").ok_or_else(|| PackageError::new(path, "缺少 password 列"))?;
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
            let password = field(&record, password_index, "password", path, row)?;
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
                password,
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

    /// 账号与 password 列都必须匹配，否则统一返回 `None`。
    pub fn authenticate(&self, student_id: &str, password: &str) -> Option<Student> {
        self.students
            .iter()
            .find(|student| student.student_id == student_id && student.password == password)
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

    fn student(student_id: &str, password: &str) -> Student {
        Student {
            student_id: student_id.to_string(),
            name: "张三".to_string(),
            school: String::new(),
            grade: String::new(),
            password: password.to_string(),
        }
    }

    #[test]
    fn parses_bom_quotes_whitespace_and_unknown_columns() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("students.csv");
        write(
            &path,
            "\u{feff}name,student_id,password,school,grade,extra\n\"张,三\", 2026001 , p1 , 第一中学 ,七年级,x\n李四,2026002,p2,,,\n",
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

        write(&path, "student_id,password\n2026001,p\n");
        assert!(Roster::load(&path)
            .unwrap_err()
            .to_string()
            .contains("name"));

        write(&path, "name,password\n张三,p\n");
        assert!(Roster::load(&path)
            .unwrap_err()
            .to_string()
            .contains("student_id"));

        write(&path, "name,student_id\n张三,2026001\n");
        assert!(Roster::load(&path)
            .unwrap_err()
            .to_string()
            .contains("password"));

        write(&path, "name,student_id,password\n,2026001,p\n");
        assert!(Roster::load(&path).is_err());

        write(&path, "name,student_id,password\n张三,,p\n");
        assert!(Roster::load(&path).is_err());

        write(&path, "name,student_id,password\n张三,2026001,\n");
        assert!(Roster::load(&path).is_err());

        write(&path, "name,student_id,password\n, ,\n");
        assert!(Roster::load(&path).is_err());
    }

    #[test]
    fn rejects_long_fields_and_duplicate_student_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("students.csv");

        write(
            &path,
            &format!("name,student_id,password\n{},2026001,p\n", "x".repeat(65)),
        );
        assert!(Roster::load(&path).is_err());

        write(
            &path,
            &format!("name,student_id,password\n张三,{},p\n", "1".repeat(65)),
        );
        assert!(Roster::load(&path).is_err());

        write(
            &path,
            "name,student_id,password,school\n张三,2026001,p1,第一中学\n李四,2026001,p2,第二中学\n",
        );
        let error = Roster::load(&path).unwrap_err().to_string();
        assert!(error.contains("ID 重复"));
    }

    #[test]
    fn rejects_empty_roster() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("students.csv");
        write(&path, "name,student_id,password\n");
        assert!(Roster::load(&path)
            .unwrap_err()
            .to_string()
            .contains("名单为空"));
    }

    #[test]
    fn authenticates_with_password_column() {
        let roster = Roster {
            students: vec![
                student("110101200001011234", "a1b2c3d4"),
                student("110101200001015678", "e5f6g7h8"),
            ],
        };

        assert_eq!(
            roster.authenticate("110101200001011234", "a1b2c3d4"),
            Some(roster.students[0].clone())
        );
        // 拿别人的口令登自己的 ID 也不行。
        assert_eq!(roster.authenticate("110101200001011234", "e5f6g7h8"), None);
        assert_eq!(roster.authenticate("110101200001019999", "a1b2c3d4"), None);
        // 大小写敏感：以身份证号作 ID 时末位可能是 X。
        assert_eq!(roster.authenticate("11010120000101123X", "a1b2c3d4"), None);
    }

    #[test]
    fn debug_redacts_password() {
        let student = student("2026001", "secret");
        let text = format!("{student:?}");
        assert!(!text.contains("secret"));
        assert!(text.contains("***"));
    }
}
