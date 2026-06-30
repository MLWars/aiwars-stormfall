//! Control REST API (port `AIWARS_CONTROL_PORT`) — a faithful copy of the chess
//! warden's `control.rs`, typed to `Match<Stormfall>`. The World-Manager proxies
//! `GET /status`, `POST /start`, `POST /stop` exactly as for any minigame world.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use tokio::sync::Mutex;

use protocol::{Phase, WorldStatus};

use crate::stormfall::Stormfall;
use aiwars_mcp_warden::game::Match;

#[derive(Clone)]
pub struct ControlState {
    pub game: Arc<Mutex<Match<Stormfall>>>,
    pub match_id: String,
    pub started_at: Arc<Mutex<Option<i64>>>,
}

fn now_epoch() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

async fn status(State(st): State<ControlState>) -> Json<WorldStatus> {
    let m = st.game.lock().await;
    let started_at = *st.started_at.lock().await;
    let phase = if m.is_resolved() { Phase::Finished } else { Phase::Live };
    Json(WorldStatus {
        instance_id: String::new(),
        match_id: st.match_id.clone(),
        phase,
        created_at: 0,
        started_at,
        deadline_at: 0,
        time_limit_secs: 0,
        elapsed_secs: 0,
        champions: vec![],
        result: m.result(),
        minigame: Some(m.state_json()),
        view_url: None,
    })
}

async fn start(State(st): State<ControlState>) -> Json<Value> {
    {
        let mut m = st.game.lock().await;
        m.start();
    }
    let mut started = st.started_at.lock().await;
    if started.is_none() {
        *started = Some(now_epoch());
    }
    Json(json!({ "ok": true }))
}

async fn stop(State(st): State<ControlState>) -> Json<Value> {
    let mut m = st.game.lock().await;
    m.abort();
    Json(json!({ "ok": true }))
}

pub fn build_control_router(state: ControlState) -> Router {
    Router::new()
        .route("/status", get(status))
        .route("/start", post(start))
        .route("/stop", post(stop))
        .with_state(state)
}
