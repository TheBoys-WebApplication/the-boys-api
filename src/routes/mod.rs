use axum::{
    routing::{delete, get, post},
    Router,
};

use crate::AppState;

mod auth;
mod groups;

pub fn router() -> Router<AppState> {
    Router::new()
        // Auth
        .route("/auth/register", post(auth::register))
        .route("/auth/login", post(auth::login))
        .route("/auth/logout", post(auth::logout))
        .route("/auth/me", get(auth::me))
        // Groups
        .route("/groups", post(groups::create_group).get(groups::list_groups))
        .route("/groups/join", post(groups::join_group))
        .route(
            "/groups/:id",
            get(groups::get_group)
                .put(groups::update_group)
                .delete(groups::delete_group),
        )
        .route("/groups/:id/invite/regenerate", post(groups::regenerate_invite))
        .route("/groups/:id/members", get(groups::list_members))
        .route("/groups/:id/members/:user_id", delete(groups::remove_member))
}
