use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::collections::HashMap;
use uuid::Uuid;

use crate::{error::AppError, middleware::auth::AuthUser, AppState};

// ── Request types ─────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct CreateExpenseRequest {
    pub description: String,
    pub amount: f64,
    /// Who paid. Defaults to the calling user.
    pub paid_by: Option<Uuid>,
    /// Which members share the cost. Defaults to all trip members.
    pub split_with: Option<Vec<Uuid>>,
}

#[derive(Deserialize)]
pub struct UpdateExpenseRequest {
    pub description: Option<String>,
    pub amount: Option<f64>,
    pub paid_by: Option<Uuid>,
    /// If provided, replaces the entire split list.
    pub split_with: Option<Vec<Uuid>>,
}

#[derive(Deserialize)]
pub struct CreateSettlementRequest {
    /// Who is sending the money. Defaults to the calling user.
    pub paid_by: Option<Uuid>,
    pub paid_to: Uuid,
    pub amount: f64,
    pub note: Option<String>,
}

// ── Response types ────────────────────────────────────────────────────────────

#[derive(Serialize, Clone)]
pub struct SplitResponse {
    pub user_id: Uuid,
    pub display_name: String,
    pub amount: f64,
}

#[derive(Serialize)]
pub struct ExpenseResponse {
    pub id: Uuid,
    pub trip_id: Uuid,
    pub paid_by: Uuid,
    pub paid_by_name: String,
    pub description: String,
    pub amount: f64,
    pub splits: Vec<SplitResponse>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Serialize)]
pub struct BalanceResponse {
    pub user_id: Uuid,
    pub display_name: String,
    /// Total expense amount this person paid.
    pub paid: f64,
    /// Total split amount assigned to this person.
    pub owed: f64,
    /// Total they have paid to others via settlements.
    pub settled_out: f64,
    /// Total they have received via settlements.
    pub settled_in: f64,
    /// net = paid - owed - settled_in + settled_out
    /// Positive → others owe them. Negative → they owe others.
    pub net: f64,
}

#[derive(Serialize)]
pub struct SettlementResponse {
    pub id: Uuid,
    pub trip_id: Uuid,
    pub paid_by: Uuid,
    pub paid_by_name: String,
    pub paid_to: Uuid,
    pub paid_to_name: String,
    pub amount: f64,
    pub note: Option<String>,
    pub created_at: DateTime<Utc>,
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Verify the caller is a member of the trip's group.
/// Returns (group_id) or NotFound.
async fn check_trip_member(
    db: &sqlx::PgPool,
    trip_id: Uuid,
    user_id: Uuid,
) -> Result<Uuid, AppError> {
    let row = sqlx::query(
        "SELECT t.group_id
         FROM trips t
         JOIN group_members gm ON gm.group_id = t.group_id AND gm.user_id = $2
         WHERE t.id = $1",
    )
    .bind(trip_id)
    .bind(user_id)
    .fetch_optional(db)
    .await?
    .ok_or(AppError::NotFound)?;

    Ok(row.get("group_id"))
}

/// Verify a UUID list are all members of a group. Returns 400 on first unknown.
async fn assert_all_members(
    db: &sqlx::PgPool,
    group_id: Uuid,
    user_ids: &[Uuid],
) -> Result<(), AppError> {
    for uid in user_ids {
        let ok: bool = sqlx::query(
            "SELECT EXISTS(SELECT 1 FROM group_members WHERE group_id = $1 AND user_id = $2)",
        )
        .bind(group_id)
        .bind(uid)
        .fetch_one(db)
        .await?
        .get(0);

        if !ok {
            return Err(AppError::BadRequest(format!(
                "user {uid} is not a member of this group"
            )));
        }
    }
    Ok(())
}

/// Fetch all member user_ids for a group.
async fn group_member_ids(db: &sqlx::PgPool, group_id: Uuid) -> Result<Vec<Uuid>, AppError> {
    let rows = sqlx::query("SELECT user_id FROM group_members WHERE group_id = $1")
        .bind(group_id)
        .fetch_all(db)
        .await?;
    Ok(rows.iter().map(|r| r.get("user_id")).collect())
}

/// Round a float to 2 decimal places.
fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// Split `amount` as evenly as possible across `n` people (integer-cent arithmetic).
/// Returns per-person amounts summing exactly to `amount`.
fn equal_splits(amount: f64, n: usize) -> Vec<f64> {
    let total_cents = (amount * 100.0).round() as i64;
    let base_cents = total_cents / n as i64;
    let extra = (total_cents % n as i64) as usize;
    (0..n)
        .map(|i| {
            let cents = base_cents + if i < extra { 1 } else { 0 };
            cents as f64 / 100.0
        })
        .collect()
}

// ── Expense handlers ──────────────────────────────────────────────────────────

pub async fn list_expenses(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(trip_id): Path<Uuid>,
) -> Result<Json<Vec<ExpenseResponse>>, AppError> {
    let auth = AuthUser::from_headers(&headers, &state.jwt_secret)?;
    check_trip_member(&state.db, trip_id, auth.user_id).await?;

    let rows = sqlx::query(
        "SELECT e.id, e.trip_id, e.paid_by, up.display_name AS paid_by_name,
                e.description, e.amount, e.created_at, e.updated_at,
                es.user_id AS split_user_id, us.display_name AS split_user_name,
                es.amount AS split_amount
         FROM expenses e
         JOIN users up ON up.id = e.paid_by
         LEFT JOIN expense_splits es ON es.expense_id = e.id
         LEFT JOIN users us ON us.id = es.user_id
         WHERE e.trip_id = $1
         ORDER BY e.created_at DESC, e.id, es.user_id",
    )
    .bind(trip_id)
    .fetch_all(&state.db)
    .await?;

    // Group rows by expense, preserving created_at DESC order.
    let mut order: Vec<Uuid> = Vec::new();
    let mut map: HashMap<Uuid, ExpenseResponse> = HashMap::new();

    for row in &rows {
        let expense_id: Uuid = row.get("id");
        map.entry(expense_id).or_insert_with(|| {
            order.push(expense_id);
            ExpenseResponse {
                id: expense_id,
                trip_id: row.get("trip_id"),
                paid_by: row.get("paid_by"),
                paid_by_name: row.get("paid_by_name"),
                description: row.get("description"),
                amount: row.get("amount"),
                splits: vec![],
                created_at: row.get("created_at"),
                updated_at: row.get("updated_at"),
            }
        });
        let split_uid: Option<Uuid> = row.try_get("split_user_id").ok().flatten();
        if let (Some(uid), Some(expense)) = (split_uid, map.get_mut(&expense_id)) {
            expense.splits.push(SplitResponse {
                user_id: uid,
                display_name: row.get("split_user_name"),
                amount: row.get("split_amount"),
            });
        }
    }

    let expenses = order.into_iter().filter_map(|id| map.remove(&id)).collect();
    Ok(Json(expenses))
}

pub async fn create_expense(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(trip_id): Path<Uuid>,
    Json(body): Json<CreateExpenseRequest>,
) -> Result<Json<ExpenseResponse>, AppError> {
    let auth = AuthUser::from_headers(&headers, &state.jwt_secret)?;
    let group_id = check_trip_member(&state.db, trip_id, auth.user_id).await?;

    // Validate input.
    if body.description.trim().is_empty() {
        return Err(AppError::BadRequest("description is required".into()));
    }
    if body.amount <= 0.0 {
        return Err(AppError::BadRequest("amount must be greater than zero".into()));
    }

    let paid_by = body.paid_by.unwrap_or(auth.user_id);
    assert_all_members(&state.db, group_id, &[paid_by]).await?;

    // Resolve split list.
    let split_ids = match body.split_with {
        Some(ref ids) if !ids.is_empty() => {
            assert_all_members(&state.db, group_id, ids).await?;
            ids.clone()
        }
        _ => group_member_ids(&state.db, group_id).await?,
    };

    if split_ids.is_empty() {
        return Err(AppError::BadRequest("split list cannot be empty".into()));
    }

    let amounts = equal_splits(body.amount, split_ids.len());

    let mut tx = state.db.begin().await?;

    let expense_row = sqlx::query(
        "INSERT INTO expenses (trip_id, paid_by, description, amount)
         VALUES ($1, $2, $3, $4)
         RETURNING id, trip_id, paid_by, description, amount, created_at, updated_at",
    )
    .bind(trip_id)
    .bind(paid_by)
    .bind(body.description.trim())
    .bind(body.amount)
    .fetch_one(&mut *tx)
    .await?;

    let expense_id: Uuid = expense_row.get("id");

    for (uid, amt) in split_ids.iter().zip(amounts.iter()) {
        sqlx::query(
            "INSERT INTO expense_splits (expense_id, user_id, amount) VALUES ($1, $2, $3)",
        )
        .bind(expense_id)
        .bind(uid)
        .bind(amt)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;

    // Fetch paid_by display name.
    let paid_by_name: String =
        sqlx::query("SELECT display_name FROM users WHERE id = $1")
            .bind(paid_by)
            .fetch_one(&state.db)
            .await?
            .get("display_name");

    // Fetch display names for split users.
    let mut splits = Vec::with_capacity(split_ids.len());
    for (uid, amt) in split_ids.iter().zip(amounts.iter()) {
        let name: String =
            sqlx::query("SELECT display_name FROM users WHERE id = $1")
                .bind(uid)
                .fetch_one(&state.db)
                .await?
                .get("display_name");
        splits.push(SplitResponse {
            user_id: *uid,
            display_name: name,
            amount: *amt,
        });
    }

    Ok(Json(ExpenseResponse {
        id: expense_id,
        trip_id: expense_row.get("trip_id"),
        paid_by,
        paid_by_name,
        description: expense_row.get("description"),
        amount: expense_row.get("amount"),
        splits,
        created_at: expense_row.get("created_at"),
        updated_at: expense_row.get("updated_at"),
    }))
}

pub async fn update_expense(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(expense_id): Path<Uuid>,
    Json(body): Json<UpdateExpenseRequest>,
) -> Result<Json<ExpenseResponse>, AppError> {
    let auth = AuthUser::from_headers(&headers, &state.jwt_secret)?;

    // Check membership and get trip/group context.
    let ctx_row = sqlx::query(
        "SELECT e.trip_id, t.group_id, e.paid_by AS original_paid_by
         FROM expenses e
         JOIN trips t ON t.id = e.trip_id
         JOIN group_members gm ON gm.group_id = t.group_id AND gm.user_id = $2
         WHERE e.id = $1",
    )
    .bind(expense_id)
    .bind(auth.user_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;

    let _trip_id: Uuid = ctx_row.get("trip_id");
    let group_id: Uuid = ctx_row.get("group_id");
    let original_paid_by: Uuid = ctx_row.get("original_paid_by");

    // Only original payer or group leader may edit.
    let is_leader: bool = sqlx::query(
        "SELECT EXISTS(SELECT 1 FROM groups WHERE id = $1 AND leader_id = $2)",
    )
    .bind(group_id)
    .bind(auth.user_id)
    .fetch_one(&state.db)
    .await?
    .get(0);

    if auth.user_id != original_paid_by && !is_leader {
        return Err(AppError::Unauthorized);
    }

    // Validate optional fields.
    if let Some(ref d) = body.description {
        if d.trim().is_empty() {
            return Err(AppError::BadRequest("description cannot be empty".into()));
        }
    }
    if let Some(a) = body.amount {
        if a <= 0.0 {
            return Err(AppError::BadRequest("amount must be greater than zero".into()));
        }
    }

    let new_paid_by = body.paid_by.unwrap_or(original_paid_by);
    if body.paid_by.is_some() {
        assert_all_members(&state.db, group_id, &[new_paid_by]).await?;
    }

    let mut tx = state.db.begin().await?;

    // Update the expense record.
    let updated_row = sqlx::query(
        "UPDATE expenses
         SET description = COALESCE($1, description),
             amount      = COALESCE($2, amount),
             paid_by     = $3,
             updated_at  = NOW()
         WHERE id = $4
         RETURNING id, trip_id, paid_by, description, amount, created_at, updated_at",
    )
    .bind(&body.description)
    .bind(body.amount)
    .bind(new_paid_by)
    .bind(expense_id)
    .fetch_one(&mut *tx)
    .await?;

    let final_amount: f64 = updated_row.get("amount");

    // If split_with provided, replace splits entirely.
    let splits = if let Some(ref split_ids) = body.split_with {
        if split_ids.is_empty() {
            return Err(AppError::BadRequest("split list cannot be empty".into()));
        }
        assert_all_members(&state.db, group_id, split_ids).await?;

        sqlx::query("DELETE FROM expense_splits WHERE expense_id = $1")
            .bind(expense_id)
            .execute(&mut *tx)
            .await?;

        let amounts = equal_splits(final_amount, split_ids.len());
        for (uid, amt) in split_ids.iter().zip(amounts.iter()) {
            sqlx::query(
                "INSERT INTO expense_splits (expense_id, user_id, amount) VALUES ($1, $2, $3)",
            )
            .bind(expense_id)
            .bind(uid)
            .bind(amt)
            .execute(&mut *tx)
            .await?;
        }
        split_ids.iter().zip(amounts.iter()).collect::<Vec<_>>()
            .iter()
            .map(|(uid, amt)| ((**uid), (**amt)))
            .collect::<Vec<(Uuid, f64)>>()
    } else if body.amount.is_some() {
        // Amount changed but split list unchanged — recalculate existing splits.
        let existing_users: Vec<Uuid> = sqlx::query(
            "SELECT user_id FROM expense_splits WHERE expense_id = $1 ORDER BY created_at",
        )
        .bind(expense_id)
        .fetch_all(&mut *tx)
        .await?
        .iter()
        .map(|r| r.get("user_id"))
        .collect();

        sqlx::query("DELETE FROM expense_splits WHERE expense_id = $1")
            .bind(expense_id)
            .execute(&mut *tx)
            .await?;

        let amounts = equal_splits(final_amount, existing_users.len());
        for (uid, amt) in existing_users.iter().zip(amounts.iter()) {
            sqlx::query(
                "INSERT INTO expense_splits (expense_id, user_id, amount) VALUES ($1, $2, $3)",
            )
            .bind(expense_id)
            .bind(uid)
            .bind(amt)
            .execute(&mut *tx)
            .await?;
        }
        existing_users.iter().zip(amounts.iter())
            .map(|(uid, amt)| (*uid, *amt))
            .collect()
    } else {
        // No amount/split changes — fetch existing splits for the response.
        sqlx::query(
            "SELECT user_id, amount FROM expense_splits WHERE expense_id = $1 ORDER BY created_at",
        )
        .bind(expense_id)
        .fetch_all(&mut *tx)
        .await?
        .iter()
        .map(|r| (r.get::<Uuid, _>("user_id"), r.get::<f64, _>("amount")))
        .collect()
    };

    tx.commit().await?;

    // Build response — fetch display names.
    let paid_by_name: String =
        sqlx::query("SELECT display_name FROM users WHERE id = $1")
            .bind(new_paid_by)
            .fetch_one(&state.db)
            .await?
            .get("display_name");

    let mut split_responses = Vec::with_capacity(splits.len());
    for (uid, amt) in &splits {
        let name: String =
            sqlx::query("SELECT display_name FROM users WHERE id = $1")
                .bind(uid)
                .fetch_one(&state.db)
                .await?
                .get("display_name");
        split_responses.push(SplitResponse {
            user_id: *uid,
            display_name: name,
            amount: *amt,
        });
    }

    Ok(Json(ExpenseResponse {
        id: updated_row.get("id"),
        trip_id: updated_row.get("trip_id"),
        paid_by: updated_row.get("paid_by"),
        paid_by_name,
        description: updated_row.get("description"),
        amount: updated_row.get("amount"),
        splits: split_responses,
        created_at: updated_row.get("created_at"),
        updated_at: updated_row.get("updated_at"),
    }))
}

pub async fn delete_expense(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(expense_id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let auth = AuthUser::from_headers(&headers, &state.jwt_secret)?;

    let ctx_row = sqlx::query(
        "SELECT e.paid_by, t.group_id
         FROM expenses e
         JOIN trips t ON t.id = e.trip_id
         JOIN group_members gm ON gm.group_id = t.group_id AND gm.user_id = $2
         WHERE e.id = $1",
    )
    .bind(expense_id)
    .bind(auth.user_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;

    let paid_by: Uuid = ctx_row.get("paid_by");
    let group_id: Uuid = ctx_row.get("group_id");

    let is_leader: bool = sqlx::query(
        "SELECT EXISTS(SELECT 1 FROM groups WHERE id = $1 AND leader_id = $2)",
    )
    .bind(group_id)
    .bind(auth.user_id)
    .fetch_one(&state.db)
    .await?
    .get(0);

    if auth.user_id != paid_by && !is_leader {
        return Err(AppError::Unauthorized);
    }

    sqlx::query("DELETE FROM expenses WHERE id = $1")
        .bind(expense_id)
        .execute(&state.db)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

// ── Balance handler ───────────────────────────────────────────────────────────

pub async fn get_balances(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(trip_id): Path<Uuid>,
) -> Result<Json<Vec<BalanceResponse>>, AppError> {
    let auth = AuthUser::from_headers(&headers, &state.jwt_secret)?;
    check_trip_member(&state.db, trip_id, auth.user_id).await?;

    let rows = sqlx::query(
        "SELECT
             gm.user_id,
             u.display_name,
             COALESCE(ep.paid,         0.0) AS paid,
             COALESCE(es2.owed,        0.0) AS owed,
             COALESCE(sp.settled_out,  0.0) AS settled_out,
             COALESCE(sr.settled_in,   0.0) AS settled_in,
             COALESCE(ep.paid,         0.0)
               - COALESCE(es2.owed,   0.0)
               - COALESCE(sr.settled_in,  0.0)
               + COALESCE(sp.settled_out, 0.0) AS net
         FROM group_members gm
         JOIN trips t ON t.id = $1
         JOIN users u ON u.id = gm.user_id
         LEFT JOIN (
             SELECT paid_by AS user_id, SUM(amount) AS paid
             FROM expenses WHERE trip_id = $1
             GROUP BY paid_by
         ) ep ON ep.user_id = gm.user_id
         LEFT JOIN (
             SELECT es.user_id, SUM(es.amount) AS owed
             FROM expense_splits es
             JOIN expenses e ON e.id = es.expense_id
             WHERE e.trip_id = $1
             GROUP BY es.user_id
         ) es2 ON es2.user_id = gm.user_id
         LEFT JOIN (
             SELECT paid_by AS user_id, SUM(amount) AS settled_out
             FROM settlements WHERE trip_id = $1
             GROUP BY paid_by
         ) sp ON sp.user_id = gm.user_id
         LEFT JOIN (
             SELECT paid_to AS user_id, SUM(amount) AS settled_in
             FROM settlements WHERE trip_id = $1
             GROUP BY paid_to
         ) sr ON sr.user_id = gm.user_id
         WHERE gm.group_id = t.group_id
         ORDER BY net DESC",
    )
    .bind(trip_id)
    .fetch_all(&state.db)
    .await?;

    let balances = rows
        .iter()
        .map(|r| {
            let net: f64 = r.get("net");
            BalanceResponse {
                user_id: r.get("user_id"),
                display_name: r.get("display_name"),
                paid: round2(r.get("paid")),
                owed: round2(r.get("owed")),
                settled_out: round2(r.get("settled_out")),
                settled_in: round2(r.get("settled_in")),
                net: round2(net),
            }
        })
        .collect();

    Ok(Json(balances))
}

// ── Settlement handlers ───────────────────────────────────────────────────────

pub async fn list_settlements(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(trip_id): Path<Uuid>,
) -> Result<Json<Vec<SettlementResponse>>, AppError> {
    let auth = AuthUser::from_headers(&headers, &state.jwt_secret)?;
    check_trip_member(&state.db, trip_id, auth.user_id).await?;

    let rows = sqlx::query(
        "SELECT s.id, s.trip_id, s.paid_by, ub.display_name AS paid_by_name,
                s.paid_to, ut.display_name AS paid_to_name,
                s.amount, s.note, s.created_at
         FROM settlements s
         JOIN users ub ON ub.id = s.paid_by
         JOIN users ut ON ut.id = s.paid_to
         WHERE s.trip_id = $1
         ORDER BY s.created_at DESC",
    )
    .bind(trip_id)
    .fetch_all(&state.db)
    .await?;

    let settlements = rows
        .iter()
        .map(|r| SettlementResponse {
            id: r.get("id"),
            trip_id: r.get("trip_id"),
            paid_by: r.get("paid_by"),
            paid_by_name: r.get("paid_by_name"),
            paid_to: r.get("paid_to"),
            paid_to_name: r.get("paid_to_name"),
            amount: round2(r.get("amount")),
            note: r.get("note"),
            created_at: r.get("created_at"),
        })
        .collect();

    Ok(Json(settlements))
}

pub async fn create_settlement(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(trip_id): Path<Uuid>,
    Json(body): Json<CreateSettlementRequest>,
) -> Result<Json<SettlementResponse>, AppError> {
    let auth = AuthUser::from_headers(&headers, &state.jwt_secret)?;
    let group_id = check_trip_member(&state.db, trip_id, auth.user_id).await?;

    if body.amount <= 0.0 {
        return Err(AppError::BadRequest("amount must be greater than zero".into()));
    }

    let paid_by = body.paid_by.unwrap_or(auth.user_id);

    if paid_by == body.paid_to {
        return Err(AppError::BadRequest(
            "paid_by and paid_to cannot be the same person".into(),
        ));
    }

    assert_all_members(&state.db, group_id, &[paid_by, body.paid_to]).await?;

    let row = sqlx::query(
        "INSERT INTO settlements (trip_id, paid_by, paid_to, amount, note)
         VALUES ($1, $2, $3, $4, $5)
         RETURNING id, trip_id, paid_by, paid_to, amount, note, created_at",
    )
    .bind(trip_id)
    .bind(paid_by)
    .bind(body.paid_to)
    .bind(body.amount)
    .bind(&body.note)
    .fetch_one(&state.db)
    .await?;

    let paid_by_name: String =
        sqlx::query("SELECT display_name FROM users WHERE id = $1")
            .bind(paid_by)
            .fetch_one(&state.db)
            .await?
            .get("display_name");

    let paid_to_name: String =
        sqlx::query("SELECT display_name FROM users WHERE id = $1")
            .bind(body.paid_to)
            .fetch_one(&state.db)
            .await?
            .get("display_name");

    Ok(Json(SettlementResponse {
        id: row.get("id"),
        trip_id: row.get("trip_id"),
        paid_by,
        paid_by_name,
        paid_to: row.get("paid_to"),
        paid_to_name,
        amount: round2(row.get("amount")),
        note: row.get("note"),
        created_at: row.get("created_at"),
    }))
}

pub async fn delete_settlement(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(settlement_id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let auth = AuthUser::from_headers(&headers, &state.jwt_secret)?;

    // Only the person who recorded the payment (paid_by) or group leader can delete.
    let ctx_row = sqlx::query(
        "SELECT s.paid_by, t.group_id
         FROM settlements s
         JOIN trips t ON t.id = s.trip_id
         JOIN group_members gm ON gm.group_id = t.group_id AND gm.user_id = $2
         WHERE s.id = $1",
    )
    .bind(settlement_id)
    .bind(auth.user_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;

    let paid_by: Uuid = ctx_row.get("paid_by");
    let group_id: Uuid = ctx_row.get("group_id");

    let is_leader: bool = sqlx::query(
        "SELECT EXISTS(SELECT 1 FROM groups WHERE id = $1 AND leader_id = $2)",
    )
    .bind(group_id)
    .bind(auth.user_id)
    .fetch_one(&state.db)
    .await?
    .get(0);

    if auth.user_id != paid_by && !is_leader {
        return Err(AppError::Unauthorized);
    }

    sqlx::query("DELETE FROM settlements WHERE id = $1")
        .bind(settlement_id)
        .execute(&state.db)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}
