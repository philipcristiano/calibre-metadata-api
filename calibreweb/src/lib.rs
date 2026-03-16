use axum::{
    Json, Router,
    extract::{FromRef, State},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqlitePool;

#[derive(Clone, Debug, Deserialize)]
pub struct CalibreWebConfig {
    pub database_url: String,
}

#[derive(FromRef, Clone, Debug)]
pub struct CWState {
    pub db: SqlitePool,
}

// Schema from calibre-web
// CREATE TABLE shelf (
// 	id INTEGER NOT NULL,
// 	uuid VARCHAR,
// 	name VARCHAR,
// 	is_public INTEGER,
// 	user_id INTEGER,
// 	kobo_sync BOOLEAN,
// 	created DATETIME,
// 	last_modified DATETIME,
// 	PRIMARY KEY (id),
// 	FOREIGN KEY(user_id) REFERENCES user (id)
// );

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Shelf {
    id: i64,
    name: Option<String>,
    user_id: Option<i64>,
}

pub async fn get_shelves(app_state: &CWState) -> Result<Vec<Shelf>, anyhow::Error> {
    let recs: Vec<Shelf> = sqlx::query_as!(
        Shelf,
        r#"
            SELECT id,
                   name,
                   user_id
            FROM shelf
        "#
    )
    .fetch_all(&app_state.db)
    .await?;
    Ok(recs)
}
