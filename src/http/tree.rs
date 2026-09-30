use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::error::ApiResult;
use super::AppState;
use crate::domain::node::{FileVersion, Node, NodeEntry};

#[derive(Deserialize)]
pub struct ListQuery {
    pub parent: Option<Uuid>,
}

#[derive(Serialize)]
pub struct Crumb {
    pub id: Uuid,
    pub name: String,
}

#[derive(Serialize)]
pub struct Listing {
    pub session: String,
    pub parent: Option<Uuid>,
    pub path: Vec<Crumb>,
    pub items: Vec<NodeEntry>,
}

#[derive(Deserialize)]
pub struct NewFolder {
    pub parent: Option<Uuid>,
    pub name: String,
}

#[derive(Deserialize)]
pub struct PatchNode {
    pub name: Option<String>,
    pub note: Option<String>,
}

pub async fn list(
    State(state): State<AppState>,
    Path(code): Path<String>,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<Listing>> {
    let session = state.sessions.open(&code).await?;
    let items = state.tree.list(&session.code, q.parent).await?;
    let path = state
        .tree
        .path(&session.code, q.parent)
        .await?
        .into_iter()
        .map(|n| Crumb {
            id: n.id,
            name: n.name,
        })
        .collect();
    Ok(Json(Listing {
        session: session.code,
        parent: q.parent,
        path,
        items,
    }))
}

pub async fn create_folder(
    State(state): State<AppState>,
    Path(code): Path<String>,
    Json(body): Json<NewFolder>,
) -> ApiResult<(StatusCode, Json<Node>)> {
    let session = state.sessions.open(&code).await?;
    let node = state
        .tree
        .create_folder(&session.code, body.parent, &body.name)
        .await?;
    Ok((StatusCode::CREATED, Json(node)))
}

pub async fn patch_node(
    State(state): State<AppState>,
    Path((code, id)): Path<(String, Uuid)>,
    Json(body): Json<PatchNode>,
) -> ApiResult<Json<Node>> {
    let session = state.sessions.open(&code).await?;
    let node = state
        .tree
        .update(
            &session.code,
            id,
            body.name.as_deref(),
            body.note.as_deref(),
        )
        .await?;
    Ok(Json(node))
}

pub async fn delete_node(
    State(state): State<AppState>,
    Path((code, id)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    let session = state.sessions.open(&code).await?;
    state.tree.delete(&session.code, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn versions(
    State(state): State<AppState>,
    Path((code, id)): Path<(String, Uuid)>,
) -> ApiResult<Json<Vec<FileVersion>>> {
    let session = state.sessions.open(&code).await?;
    Ok(Json(state.tree.versions(&session.code, id).await?))
}

pub async fn add_tag(
    State(state): State<AppState>,
    Path((code, version, tag)): Path<(String, Uuid, String)>,
) -> ApiResult<Json<FileVersion>> {
    let session = state.sessions.open(&code).await?;
    Ok(Json(
        state.tree.add_tag(&session.code, version, &tag).await?,
    ))
}

pub async fn remove_tag(
    State(state): State<AppState>,
    Path((code, version, tag)): Path<(String, Uuid, String)>,
) -> ApiResult<StatusCode> {
    let session = state.sessions.open(&code).await?;
    state.tree.remove_tag(&session.code, version, &tag).await?;
    Ok(StatusCode::NO_CONTENT)
}
