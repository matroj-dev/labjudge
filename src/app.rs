use axum::Router;

use crate::app_state::AppState;
use crate::routes;

pub fn build_router(state: AppState) -> Router {
    routes::router().with_state(state)
}
