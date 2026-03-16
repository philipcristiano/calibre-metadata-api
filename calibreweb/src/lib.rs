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
// From calibre-web app.db schema
// CREATE TABLE book_shelf_link (
// 	id INTEGER NOT NULL,
// 	book_id INTEGER,
// 	"order" INTEGER,
// 	shelf INTEGER,
// 	date_added DATETIME,
// 	PRIMARY KEY (id),
// 	FOREIGN KEY(shelf) REFERENCES shelf (id)
// );

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BookShelfLink {
    id: i64,
    pub book_id: Option<i64>,
}

pub async fn get_shelf_book_ids(
    app_state: &CWState,
    shelf_id: i32,
) -> Result<Vec<BookShelfLink>, anyhow::Error> {
    let recs = sqlx::query_as!(
        BookShelfLink,
        r#"
            SELECT id,
               book_id
            FROM book_shelf_link
            WHERE
                shelf = ?1

        "#,
        shelf_id
    )
    .fetch_all(&app_state.db)
    .await?;
    Ok(recs)
}
