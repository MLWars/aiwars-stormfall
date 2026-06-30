//! `aiwars-mcp-stormfall` binary entrypoint — mirrors the chess warden's `main.rs`
//! exactly, but builds a `Match<Stormfall>` instead of `Match<Chess>`.
//!
//! Environment:
//! - `AIWARS_MATCH` — JSON of [`protocol::MatchConfig`] (settings + agents).
//! - `AIWARS_MATCH_ID` — optional match id, echoed in `GET /status`.
//! - `AIWARS_CONTROL_PORT` / `AIWARS_MCP_PORT` / `AIWARS_VIEW_PORT` — ports.
//! - `AIWARS_VIEW_DIR` — baked spectator SPA dir (default `/srv/view`).
//! - `AIWARS_APP_ORIGIN` — site origin allowed to frame the view.

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::sync::Mutex;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

use aiwars_mcp_stormfall::control::{build_control_router, ControlState};
use aiwars_mcp_stormfall::mcp::build_mcp_router;
use aiwars_mcp_stormfall::stormfall::Stormfall;
use aiwars_mcp_stormfall::view::{build_view_router, ViewState};
use aiwars_mcp_warden::game::Match;
use protocol::MatchConfig;

fn env_port(name: &str, default: u16) -> u16 {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,rmcp=debug")),
        )
        .init();

    let match_id = std::env::var("AIWARS_MATCH_ID").unwrap_or_default();
    let control_port = env_port("AIWARS_CONTROL_PORT", 8080);
    let mcp_port = env_port("AIWARS_MCP_PORT", 9090);
    let view_port = env_port("AIWARS_VIEW_PORT", 8090);
    let view_dir = std::env::var("AIWARS_VIEW_DIR").unwrap_or_else(|_| "/srv/view".to_string());
    let view_app_origin = std::env::var("AIWARS_APP_ORIGIN").unwrap_or_default();

    let raw = std::env::var("AIWARS_MATCH")
        .context("AIWARS_MATCH env var is required (JSON of protocol::MatchConfig)")?;
    let cfg: MatchConfig =
        serde_json::from_str(&raw).context("AIWARS_MATCH is not a valid MatchConfig JSON")?;

    let handles: Vec<String> = cfg.agents.iter().map(|a| a.handle.clone()).collect();
    let token_hashes: Vec<String> = cfg.agents.iter().map(|a| a.token_hash.clone()).collect();

    let game = Match::<Stormfall>::new(handles.clone(), &cfg.settings)
        .map_err(|e| anyhow::anyhow!("failed to build match: {e}"))?;
    let shared = Arc::new(Mutex::new(game));

    info!(agents = handles.len(), %match_id, control_port, mcp_port, "stormfall referee starting");

    let control_state = ControlState {
        game: shared.clone(),
        match_id: match_id.clone(),
        started_at: Arc::new(Mutex::new(None)),
    };
    let control_router = build_control_router(control_state);
    let control_addr = SocketAddr::from(([0, 0, 0, 0], control_port));

    let mcp_router = build_mcp_router(shared.clone(), token_hashes);
    let mcp_addr = SocketAddr::from(([0, 0, 0, 0], mcp_port));

    let view_router =
        build_view_router(ViewState { game: shared.clone() }, &view_dir, &view_app_origin);
    let view_addr = SocketAddr::from(([0, 0, 0, 0], view_port));

    let control_listener = tokio::net::TcpListener::bind(control_addr)
        .await
        .with_context(|| format!("binding control REST on {control_addr}"))?;
    let mcp_listener = tokio::net::TcpListener::bind(mcp_addr)
        .await
        .with_context(|| format!("binding MCP server on {mcp_addr}"))?;
    let view_listener = tokio::net::TcpListener::bind(view_addr)
        .await
        .with_context(|| format!("binding view server on {view_addr}"))?;

    info!("control REST on {control_addr} · MCP on {mcp_addr} · view on {view_addr} (dir {view_dir})");

    let control_srv = axum::serve(control_listener, control_router);
    let mcp_srv = axum::serve(mcp_listener, mcp_router);
    let view_srv = axum::serve(view_listener, view_router);

    tokio::select! {
        r = control_srv => { if let Err(e) = r { error!("control REST server exited: {e}"); } }
        r = mcp_srv => { if let Err(e) = r { error!("MCP server exited: {e}"); } }
        r = view_srv => { if let Err(e) = r { error!("view server exited: {e}"); } }
    }
    Ok(())
}
