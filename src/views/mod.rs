use askama::Template;

pub mod admin;
pub mod fragments;
pub mod student;

#[derive(Template)]
#[template(path = "base.html")]
pub struct BaseTemplate;

#[derive(Template)]
#[template(path = "error.html")]
pub struct ErrorTemplate {
    pub status: u16,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_has_only_local_assets() {
        let html = BaseTemplate.render().unwrap();
        assert!(html.contains("/static/app.css"));
        assert!(html.contains("/static/app.js"));
        assert!(!html.contains("http://"));
        assert!(!html.contains("https://"));
    }

    #[test]
    fn error_renders_chinese_message() {
        let html = ErrorTemplate {
            status: 404,
            message: "题目不存在".to_string(),
        }
        .render()
        .unwrap();
        assert!(html.contains("404"));
        assert!(html.contains("题目不存在"));
        assert!(html.contains("返回比赛列表"));
    }
}
