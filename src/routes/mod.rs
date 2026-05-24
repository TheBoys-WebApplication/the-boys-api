use axum::{
    routing::{delete, get, post, put},
    Router,
};

use crate::AppState;

mod auth;
mod groups;
mod trips;

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
        // Trips
        .route("/groups/:id/trips", get(trips::list_trips).post(trips::create_trip))
        .route(
            "/trips/:id",
            get(trips::get_trip)
                .put(trips::update_trip)
                .delete(trips::delete_trip),
        )
        // Activities
        .route("/trips/:id/activities", get(trips::list_activities).post(trips::create_activity))
        .route(
            "/activities/:id",
            put(trips::update_activity).delete(trips::delete_activity),
        )
}
