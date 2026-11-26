use askama::Template;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use crate::views::ErrorTemplate;

#[derive(Debug)]
pub struct AppError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
    pub as_json: bool,
}

impl AppError {
    pub fn page(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            code: code_for(status),
            message: message.into(),
            as_json: false,
        }
    }

    pub fn json(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            as_json: true,
        }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::page(StatusCode::BAD_REQUEST, message)
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::page(StatusCode::FORBIDDEN, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::page(StatusCode::NOT_FOUND, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        let details = message.into();
        tracing::error!(%details, "内部错误");
        Self::page(
            StatusCode::INTERNAL_SERVER_ERROR,
            "服务器内部错误，请稍后重试",
        )
    }

    pub fn unauthorized_json(message: impl Into<String>) -> Self {
        Self::json(StatusCode::UNAUTHORIZED, "unauthorized", message)
    }

    pub fn not_found_json(message: impl Into<String>) -> Self {
        Self::json(StatusCode::NOT_FOUND, "not_found", message)
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        if self.as_json {
            return (
                self.status,
                Json(json!({
                    "error": {
                        "code": self.code,
                        "message": self.message,
                    }
                })),
            )
                .into_response();
        }

        match (ErrorTemplate {
            status: self.status.as_u16(),
            message: self.message,
        })
        .render()
        {
            Ok(html) => (self.status, axum::response::Html(html)).into_response(),
            Err(error) => {
                tracing::error!(%error, "渲染错误页失败");
                (self.status, "服务器错误").into_response()
            }
        }
    }
}

fn code_for(status: StatusCode) -> &'static str {
    match status {
        StatusCode::BAD_REQUEST => "bad_request",
        StatusCode::FORBIDDEN => "forbidden",
        StatusCode::NOT_FOUND => "not_found",
        StatusCode::UNAUTHORIZED => "unauthorized",
        StatusCode::METHOD_NOT_ALLOWED => "method_not_allowed",
        _ => "internal",
    }
}
