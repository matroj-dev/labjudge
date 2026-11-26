use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

pub struct Session {
    base: String,
    jar: PathBuf,
    response: PathBuf,
    form: PathBuf,
}

pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn text(&self) -> Result<String> {
        String::from_utf8(self.body.clone()).context("响应不是合法 UTF-8")
    }

    pub fn expect_ok(&self, label: &str) -> Result<()> {
        if self.status == 200 {
            Ok(())
        } else {
            bail!("{label} 返回 HTTP {}", self.status);
        }
    }
}

impl Session {
    pub fn new(base: &str, dir: &Path, name: &str) -> Self {
        let jar = dir.join(format!("{name}.cookies"));
        let _ = fs::write(&jar, b"");
        Self {
            base: base.to_string(),
            jar,
            response: dir.join(format!("{name}.response")),
            form: dir.join(format!("{name}.form")),
        }
    }

    pub fn get(&self, path: &str) -> Result<HttpResponse> {
        self.request("GET", path, None)
    }

    pub fn post_form(&self, path: &str, fields: &[(&str, &str)]) -> Result<HttpResponse> {
        fs::write(&self.form, form_body(fields))?;
        self.request("POST", path, Some(&self.form))
    }

    fn request(&self, method: &str, path: &str, form: Option<&Path>) -> Result<HttpResponse> {
        let mut command = Command::new("curl.exe");
        command
            .arg("--silent")
            .arg("--show-error")
            .arg("--location")
            .arg("--max-redirs")
            .arg("10")
            .arg("--connect-timeout")
            .arg("5")
            .arg("--max-time")
            .arg("30")
            .arg("--noproxy")
            .arg("*")
            .arg("--cookie")
            .arg(&self.jar)
            .arg("--cookie-jar")
            .arg(&self.jar)
            .arg("--output")
            .arg(&self.response)
            .arg("--write-out")
            .arg("%{http_code}")
            .arg("--header")
            .arg(format!("Origin: {}", self.base))
            .arg("--header")
            .arg(format!("Referer: {}", self.base))
            .arg("--url")
            .arg(format!("{}{}", self.base, path));

        if method == "POST" {
            let form = form.ok_or_else(|| anyhow::anyhow!("POST 缺少表单"))?;
            command
                .arg("--header")
                .arg("Content-Type: application/x-www-form-urlencoded")
                .arg("--data-binary")
                .arg(format!("@{}", form.display()));
        }

        let output = command
            .output()
            .context("无法执行 curl.exe；Windows 10/11 应自带 curl")?;
        if !output.status.success() {
            bail!(
                "curl 请求 {path} 失败: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        let status = String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse::<u16>()
            .with_context(|| format!("无法解析 HTTP 状态码: {}", path))?;
        let body = fs::read(&self.response).unwrap_or_default();
        Ok(HttpResponse { status, body })
    }
}

pub fn ensure_curl() -> Result<()> {
    let output = Command::new("curl.exe")
        .arg("--version")
        .output()
        .context("无法执行 curl.exe；Windows 10/11 应自带 curl")?;
    if !output.status.success() {
        bail!("curl.exe 不可用");
    }
    Ok(())
}

fn form_body(fields: &[(&str, &str)]) -> String {
    fields
        .iter()
        .map(|(key, value)| format!("{}={}", encode_component(key), encode_component(value)))
        .collect::<Vec<_>>()
        .join("&")
}

fn encode_component(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                output.push(*byte as char);
            }
            _ => output.push_str(&format!("%{byte:02X}")),
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn form_body_percent_encodes_unicode_and_newlines() {
        let body = form_body(&[("name", "张三"), ("source", "a b\nc+")]);
        assert_eq!(body, "name=%E5%BC%A0%E4%B8%89&source=a%20b%0Ac%2B");
    }

    #[test]
    fn form_body_keeps_unreserved_characters() {
        assert_eq!(form_body(&[("key", "A-1.z_~")]), "key=A-1.z_~");
    }
}
