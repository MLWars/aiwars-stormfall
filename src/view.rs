//! Read-only spectator view server (port `AIWARS_VIEW_PORT`) — a faithful copy of
//! the chess warden's `view.rs`, typed to `Match<Stormfall>`. Serves `/state.json`
//! (a pure read of the match) plus the baked SPA from `view_dir`.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::{DefaultBodyLimit, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::routing::get;
use axum::{Json, Router};
use tokio::sync::Mutex;
use tower_http::services::ServeDir;
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::timeout::TimeoutLayer;

use crate::stormfall::Stormfall;
use aiwars_mcp_warden::game::Match;

#[derive(Clone)]
pub struct ViewState {
    pub game: Arc<Mutex<Match<Stormfall>>>,
}

/// `GET /state.json` — the live battle-royale JSON the SPA polls. A pure read.
async fn state_json(State(st): State<ViewState>) -> Json<serde_json::Value> {
    Json(st.game.lock().await.state_json())
}

pub fn build_view_router(state: ViewState, view_dir: &str, app_origin: &str) -> Router {
    let frame_ancestors = if app_origin.is_empty() {
        "'self'".to_string()
    } else {
        format!("'self' {app_origin}")
    };
    let csp = format!(
        "default-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; \
         frame-ancestors {frame_ancestors}; object-src 'none'; base-uri 'none'"
    );

    let serve = ServeDir::new(view_dir).append_index_html_on_directories(true);

    Router::new()
        .route("/state.json", get(state_json))
        .fallback_service(serve)
        .with_state(state)
        .layer(DefaultBodyLimit::max(1024))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(15),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-store"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_str(&csp)
                .unwrap_or_else(|_| HeaderValue::from_static("default-src 'self'")),
        ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use std::path::PathBuf;
    use tower::ServiceExt;

    fn match_fixture() -> Arc<Mutex<Match<Stormfall>>> {
        let handles = vec!["vortex".to_string(), "bunker".to_string()];
        let m = Match::<Stormfall>::new(handles, &serde_json::json!({ "seed": 7 })).unwrap();
        Arc::new(Mutex::new(m))
    }

    fn view_dir() -> String {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("view").to_string_lossy().into_owned()
    }

    fn router() -> Router {
        build_view_router(ViewState { game: match_fixture() }, &view_dir(), "")
    }

    #[tokio::test]
    async fn state_json_is_a_readable_battle() {
        let resp = router()
            .oneshot(Request::get("/state.json").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["game"], "stormfall");
        assert!(v["players"].is_array());
    }

    #[tokio::test]
    async fn polling_does_not_mutate_the_game() {
        let r = router();
        let read = |r: Router| async move {
            let resp = r
                .oneshot(Request::get("/state.json").body(Body::empty()).unwrap())
                .await
                .unwrap();
            let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20).await.unwrap();
            let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            v["ply"].as_u64().unwrap()
        };
        assert_eq!(read(r.clone()).await, read(r.clone()).await, "reading must not advance state");
    }
}
