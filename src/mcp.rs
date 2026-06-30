//! The MCP server for the Stormfall referee. A faithful copy of the chess warden's
//! `mcp.rs`, typed to `Match<Stormfall>`: a SINGLE bearer-gated `/mcp` endpoint
//! serves all agents; the caller's seat is resolved from its bearer token. The
//! four tools (`get_state`, `legal_moves`, `make_move`, `resign`) are unchanged —
//! they call the game-agnostic `Match` methods, so the move string is just an
//! action (e.g. `hunt:rival`) instead of a UCI move.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::Request;
use axum::http::request::Parts;
use axum::http::{header::AUTHORIZATION, StatusCode};
use axum::response::Response;
use axum::Router;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, Content, ServerCapabilities, ServerInfo};
use rmcp::service::RequestContext;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{tool, tool_handler, tool_router, ErrorData, RoleServer, ServerHandler};
use schemars::JsonSchema;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

use crate::stormfall::Stormfall;
use aiwars_mcp_warden::game::Match;

/// Shared, mutex-guarded match state shared across all agents' requests.
pub type SharedMatch = Arc<Mutex<Match<Stormfall>>>;

/// `make_move` tool arguments.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct MakeMoveArgs {
    /// The action to take, one of your legal moves (e.g. "loot:crate",
    /// "hide:bunker", "rotate:eye", "hunt:rival").
    pub mv: String,
    /// The ply you observed when you decided this move (optimistic concurrency).
    pub expected_ply: u32,
}

/// The MCP server handler. ONE handler serves every seat; the caller's seat is
/// resolved per-request from its bearer token.
#[derive(Clone)]
pub struct RefereeHandler {
    state: SharedMatch,
    token_hashes: Arc<Vec<String>>,
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
}

impl RefereeHandler {
    fn new(state: SharedMatch, token_hashes: Arc<Vec<String>>) -> Self {
        Self { state, token_hashes, tool_router: Self::tool_router() }
    }

    fn tool_err(msg: String) -> CallToolResult {
        CallToolResult::error(vec![Content::text(msg)])
    }

    fn ok_json(value: serde_json::Value) -> Result<CallToolResult, ErrorData> {
        let content =
            Content::json(&value).map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![content]))
    }

    fn seat_from_ctx(&self, ctx: &RequestContext<RoleServer>) -> Result<usize, ErrorData> {
        let parts = ctx
            .extensions
            .get::<Parts>()
            .ok_or_else(|| ErrorData::internal_error("no request context on tool call", None))?;
        let token = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.strip_prefix("Bearer ").or_else(|| s.strip_prefix("bearer ")))
            .ok_or_else(|| ErrorData::invalid_params("missing bearer token", None))?;
        let presented = token_hash(token);
        self.token_hashes
            .iter()
            .position(|expected| fixed_time_eq(expected.as_bytes(), presented.as_bytes()))
            .ok_or_else(|| ErrorData::invalid_params("unknown agent token", None))
    }
}

#[tool_router]
impl RefereeHandler {
    #[tool(
        description = "Get the current isle state: your HP/gear/position, whether you're inside the safe ring, the rival's HP, the storm round and safe-zone eye, survivors left, status, winner, and your legal actions.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn get_state(&self) -> Result<CallToolResult, ErrorData> {
        tracing::info!("TOOL-CALL get_state");
        let m = self.state.lock().await;
        Self::ok_json(m.state_json())
    }

    #[tool(
        description = "List the actions you may take now (loot:crate / hide:bunker / rotate:eye / hunt:rival — hunt only if the rival is reachable) and whether it is your turn, with the current ply.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn legal_moves(
        &self,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let seat = self.seat_from_ctx(&ctx)?;
        tracing::info!(seat, "TOOL-CALL legal_moves");
        let m = self.state.lock().await;
        Self::ok_json(m.turn_info(seat))
    }

    #[tool(
        description = "Take one action. Pass the ply you saw as expected_ply. loot:crate gains gear (stronger hunt) but leaves you exposed; hide:bunker is safe and patches a little HP; rotate:eye moves toward the safe-zone eye to dodge the storm; hunt:rival strikes the rival (needs them reachable). Returns the new state on success; an error (no change) if illegal, out of turn, or stale.",
        annotations(read_only_hint = false, destructive_hint = false, open_world_hint = false)
    )]
    async fn make_move(
        &self,
        Parameters(MakeMoveArgs { mv, expected_ply }): Parameters<MakeMoveArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let seat = self.seat_from_ctx(&ctx)?;
        tracing::info!(seat, %mv, expected_ply, "TOOL-CALL make_move");
        let mut m = self.state.lock().await;
        match m.make_move(seat, &mv, expected_ply) {
            Ok(state) => Self::ok_json(state),
            Err(e) => Ok(Self::tool_err(e.to_string())),
        }
    }

    #[tool(
        description = "Resign the battle-royale (forfeit). Your rival is awarded the win.",
        annotations(read_only_hint = false, destructive_hint = false, open_world_hint = false)
    )]
    async fn resign(&self, ctx: RequestContext<RoleServer>) -> Result<CallToolResult, ErrorData> {
        let seat = self.seat_from_ctx(&ctx)?;
        tracing::info!(seat, "TOOL-CALL resign");
        let mut m = self.state.lock().await;
        let state = m.resign(seat);
        Self::ok_json(state)
    }
}

#[tool_handler]
impl ServerHandler for RefereeHandler {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "AIWars Stormfall Isle referee. Call get_state / legal_moves to observe, make_move to \
             take one action (loot:crate · hide:bunker · rotate:eye · hunt:rival, with the ply you \
             saw), resign to forfeit. A magenta storm wall contracts each round toward a hidden \
             seeded eye, draining HP outside the ring — stay in the zone, gear up, and eliminate \
             your rival or be the last one standing. Your seat is identified by your bearer token; \
             you cannot act as your rival.",
        )
    }
}

fn fixed_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// `sha256(token)` as lowercase hex.
pub fn token_hash(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

fn unauthorized() -> Response {
    Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .body(Body::from("unauthorized"))
        .expect("static 401 response is valid")
}

/// Build the MCP router: a SINGLE bearer-gated `/mcp` endpoint shared by all agents.
pub fn build_mcp_router(state: SharedMatch, token_hashes: Vec<String>) -> Router {
    let token_hashes: Arc<Vec<String>> =
        Arc::new(token_hashes.into_iter().map(|h| h.to_lowercase()).collect());

    let svc_state = state.clone();
    let svc_hashes = token_hashes.clone();
    let json_response = std::env::var("AIWARS_MCP_JSON_RESPONSE")
        .map(|v| v != "0" && !v.is_empty())
        .unwrap_or(true);
    let stateful = std::env::var("AIWARS_MCP_STATEFUL")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let server_config = StreamableHttpServerConfig::default()
        .with_json_response(json_response)
        .with_stateful_mode(stateful)
        .disable_allowed_hosts();
    tracing::info!(json_response, stateful, "MCP streamable-HTTP server config");
    let service = StreamableHttpService::new(
        move || Ok(RefereeHandler::new(svc_state.clone(), svc_hashes.clone())),
        Arc::new(LocalSessionManager::default()),
        server_config,
    );

    let auth_hashes = token_hashes.clone();
    let guarded =
        axum::middleware::from_fn(move |req: Request, next: axum::middleware::Next| {
            let auth_hashes = auth_hashes.clone();
            async move {
                let method = req.method().clone();
                let path = req.uri().path().to_string();
                let authed = req
                    .headers()
                    .get(AUTHORIZATION)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.strip_prefix("Bearer ").or_else(|| s.strip_prefix("bearer ")))
                    .map(|tok| {
                        let presented = token_hash(tok);
                        auth_hashes
                            .iter()
                            .any(|e| fixed_time_eq(e.as_bytes(), presented.as_bytes()))
                    })
                    .unwrap_or(false);
                if !authed {
                    tracing::warn!(%method, path, "MCP-REQ unauthorized");
                    return unauthorized();
                }
                let resp = next.run(req).await;
                tracing::info!(%method, path, status = %resp.status(), "MCP-REQ");
                resp
            }
        });

    Router::new().nest_service(
        "/mcp",
        tower::ServiceBuilder::new().layer(guarded).service(service),
    )
}
