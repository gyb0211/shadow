//! API 路由聚合

mod agents;
mod auth;
mod channels;
mod config;
mod logs;
mod providers;
mod status;
mod tools;
mod users;

use axum::Router;

use crate::state::GatewayState;

pub fn routes() -> Router<GatewayState> {
    Router::new()
        .merge(auth::routes())
        .merge(status::routes())
        .merge(users::routes())
        .merge(agents::routes())
        .merge(channels::routes())
        .merge(providers::routes())
        .merge(tools::routes())
        .merge(config::routes())
        .merge(logs::routes())
}
