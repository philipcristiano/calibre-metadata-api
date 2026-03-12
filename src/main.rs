use clap::Parser;
use serde::{Deserialize, Serialize};

use sqlx::sqlite::SqlitePool;

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use axum_extra::extract::Query;
use std::net::SocketAddr;

use tower_http::cors::CorsLayer;

#[derive(Parser, Debug)]
pub struct Args {
    #[arg(short, long, default_value = "127.0.0.1:3002")]
    bind_addr: String,
    #[arg(short, long, default_value = "cma.toml")]
    config_file: String,
    #[arg(short, long, value_enum, default_value = "DEBUG")]
    log_level: tracing::Level,
}

#[derive(Clone, Debug, Deserialize)]
struct AppConfig {
    database_url: String,
}

#[derive(Clone, Debug)]
struct AppState {
    db: SqlitePool,
}

fn read_app_config(path: &str) -> anyhow::Result<AppConfig> {
    use std::fs;
    let contents = fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("Could not read config file {}: {}", path, e))?;
    let config = toml::from_str(&contents)
        .map_err(|e| anyhow::anyhow!("Problems parsing config file: {}", e))?;
    Ok(config)
}

fn build_app(pool: SqlitePool) -> Router {
    let app_state = AppState { db: pool };
    Router::new()
        .route("/_health", get(health))
        .route("/v1/authors", get(get_authors))
        .route("/v1/authors/{id}", get(get_author_by_id))
        .route("/v1/books", get(get_books))
        .route("/v1/books/{id}", get(get_book_by_id))
        .route("/v1/series", get(get_series))
        .route("/v1/series/{id}", get(get_series_by_id))
        .route("/v1/tags", get(get_tags))
        .route("/v1/tags/{id}", get(get_tag_by_id))
        .with_state(app_state)
        .layer(tower_http::compression::CompressionLayer::new())
        .layer(CorsLayer::permissive())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    service_conventions::tracing::setup(args.log_level);

    let app_config = read_app_config(&args.config_file)?;

    tracing::info!("connecting to database");
    let pool = SqlitePool::connect(&app_config.database_url)
        .await
        .map_err(|e| anyhow::anyhow!("Cannot connect to database: {}", e))?;

    let app = build_app(pool).layer(service_conventions::tracing_http::trace_layer(
        tracing::Level::INFO,
    ));

    let addr: SocketAddr = args
        .bind_addr
        .parse()
        .map_err(|e| anyhow::anyhow!("Invalid bind address '{}': {}", args.bind_addr, e))?;
    tracing::info!("listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to bind to {}: {}", addr, e))?;
    axum::serve(listener, app)
        .await
        .map_err(|e| anyhow::anyhow!("Server error: {}", e))?;
    Ok(())
}

async fn health(State(app_state): State<AppState>) -> Response {
    match sqlx::query("SELECT 1").execute(&app_state.db).await {
        Ok(_) => "OK".into_response(),
        Err(_) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

const MAX_LIMIT: i64 = 1000;

fn clamp_limit(v: Option<i64>) -> i64 {
    v.unwrap_or(100).min(MAX_LIMIT).max(0)
}

fn clamp_offset(v: Option<i64>) -> i64 {
    v.unwrap_or(0).max(0)
}

#[derive(Debug, Deserialize, Default)]
struct Pagination {
    limit: Option<i64>,
    offset: Option<i64>,
}

impl Pagination {
    fn limit(&self) -> i64 { clamp_limit(self.limit) }
    fn offset(&self) -> i64 { clamp_offset(self.offset) }
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
enum SortField {
    #[default]
    Id,
    Title,
    Pubdate,
}

impl SortField {
    fn as_sql(&self) -> &'static str {
        match self {
            SortField::Id => "books.id",
            SortField::Title => "books.title",
            SortField::Pubdate => "books.pubdate",
        }
    }
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
enum SortDir {
    #[default]
    Asc,
    Desc,
}

impl SortDir {
    fn as_sql(&self) -> &'static str {
        match self {
            SortDir::Asc => "ASC",
            SortDir::Desc => "DESC",
        }
    }
}

#[derive(Debug, Deserialize, Default)]
struct BooksQuery {
    author_id: Option<i64>,
    series_id: Option<i64>,
    tag_id: Option<i64>,
    q: Option<String>,
    sort: Option<SortField>,
    sort_dir: Option<SortDir>,
    limit: Option<i64>,
    offset: Option<i64>,
}

impl BooksQuery {
    fn limit(&self) -> i64 { clamp_limit(self.limit) }
    fn offset(&self) -> i64 { clamp_offset(self.offset) }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct ApiResponse<T> {
    data: T,
}

#[derive(Debug, Clone, Deserialize, Serialize, sqlx::FromRow)]
struct Author {
    id: i64,
    name: String,
    sort: Option<String>,
    link: String,
}

/// An author embedded in a Book response.
#[derive(Debug, Clone, Deserialize, Serialize)]
struct BookAuthor {
    id: i64,
    name: String,
}

/// Raw row returned from the database for a book query.
/// `authors_json` and `tags_json` are JSON arrays produced by SQLite's
/// json_group_array and need to be deserialized into the Book response type.
#[derive(Debug, sqlx::FromRow)]
struct BookRow {
    id: i64,
    title: String,
    pubdate: Option<chrono::NaiveDateTime>,
    authors_json: Option<String>,
    tags_json: Option<String>,
    isbn: Option<String>,
    series_name: Option<String>,
    series_index: Option<f64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct Book {
    id: i64,
    title: String,
    pubdate: Option<chrono::NaiveDateTime>,
    authors: Vec<BookAuthor>,
    tags: Vec<String>,
    isbn: Option<String>,
    series_name: Option<String>,
    series_index: Option<f64>,
}

impl TryFrom<BookRow> for Book {
    type Error = serde_json::Error;

    fn try_from(row: BookRow) -> Result<Self, Self::Error> {
        let authors: Vec<BookAuthor> = row
            .authors_json
            .as_deref()
            .map(serde_json::from_str)
            .transpose()?
            .unwrap_or_default();
        let tags: Vec<String> = row
            .tags_json
            .as_deref()
            .map(serde_json::from_str)
            .transpose()?
            .unwrap_or_default();
        Ok(Book {
            id: row.id,
            title: row.title,
            pubdate: row.pubdate,
            authors,
            tags,
            isbn: row.isbn,
            series_name: row.series_name,
            series_index: row.series_index,
        })
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, sqlx::FromRow)]
struct Series {
    id: i64,
    name: String,
    sort: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, sqlx::FromRow)]
struct Tag {
    id: i64,
    name: String,
}

async fn get_authors(
    State(app_state): State<AppState>,
    Query(query): Query<Pagination>,
) -> Result<Response, AppError> {
    let recs = sqlx::query_as::<_, Author>(
        "SELECT id, name, sort, link FROM authors ORDER BY sort LIMIT ? OFFSET ?",
    )
    .bind(query.limit())
    .bind(query.offset())
    .fetch_all(&app_state.db)
    .await?;

    Ok(Json(ApiResponse { data: recs }).into_response())
}

async fn get_author_by_id(
    State(app_state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let rec =
        sqlx::query_as::<_, Author>("SELECT id, name, sort, link FROM authors WHERE id = ?")
            .bind(id)
            .fetch_optional(&app_state.db)
            .await?;

    match rec {
        Some(author) => Ok(Json(ApiResponse { data: author }).into_response()),
        None => Err(AppError::NotFound),
    }
}

// Correlated subqueries for authors and tags avoid the cartesian-product
// duplication that a JOIN would cause for books with multiple authors or tags.
const BOOKS_BASE_QUERY: &str = "
    SELECT books.id as id, title, pubdate,
           (SELECT json_group_array(json_object('id', a.id, 'name', a.name))
            FROM books_authors_link bal
            JOIN authors a ON bal.author = a.id
            WHERE bal.book = books.id) as authors_json,
           (SELECT json_group_array(t.name)
            FROM books_tags_link btl
            JOIN tags t ON btl.tag = t.id
            WHERE btl.book = books.id) as tags_json,
           i.val as isbn,
           s.name as series_name,
           books.series_index as series_index
    FROM books
    LEFT JOIN identifiers i ON i.book = books.id AND i.type = 'isbn'
    LEFT JOIN books_series_link bsl ON bsl.book = books.id
    LEFT JOIN series s ON s.id = bsl.series
";

async fn get_books(
    State(app_state): State<AppState>,
    Query(query): Query<BooksQuery>,
) -> Result<Response, AppError> {
    let mut qb: sqlx::QueryBuilder<sqlx::Sqlite> = sqlx::QueryBuilder::new(BOOKS_BASE_QUERY);
    let mut has_where = false;

    if let Some(author_id) = query.author_id {
        qb.push(if has_where { " AND " } else { " WHERE " });
        qb.push("books.id IN (SELECT book FROM books_authors_link WHERE author = ")
            .push_bind(author_id)
            .push(")");
        has_where = true;
    }

    if let Some(series_id) = query.series_id {
        qb.push(if has_where { " AND " } else { " WHERE " });
        qb.push("books.id IN (SELECT book FROM books_series_link WHERE series = ")
            .push_bind(series_id)
            .push(")");
        has_where = true;
    }

    if let Some(tag_id) = query.tag_id {
        qb.push(if has_where { " AND " } else { " WHERE " });
        qb.push("books.id IN (SELECT book FROM books_tags_link WHERE tag = ")
            .push_bind(tag_id)
            .push(")");
        has_where = true;
    }

    if let Some(ref q) = query.q {
        qb.push(if has_where { " AND " } else { " WHERE " });
        // Match title or any of the book's authors by name
        qb.push("(books.title LIKE '%' || ").push_bind(q).push(" || '%'");
        qb.push(" OR books.id IN (SELECT bal.book FROM books_authors_link bal JOIN authors a ON bal.author = a.id WHERE a.name LIKE '%' || ")
            .push_bind(q)
            .push(" || '%'))");
        has_where = true;
    }

    let _ = has_where; // all filters applied

    let sort_field = query.sort.as_ref().unwrap_or(&SortField::Id);
    let sort_dir = query.sort_dir.as_ref().unwrap_or(&SortDir::Asc);
    qb.push(format!(
        " ORDER BY {} {}",
        sort_field.as_sql(),
        sort_dir.as_sql()
    ));
    qb.push(" LIMIT ").push_bind(query.limit());
    qb.push(" OFFSET ").push_bind(query.offset());

    let rows = qb
        .build_query_as::<BookRow>()
        .fetch_all(&app_state.db)
        .await?;

    let books = rows
        .into_iter()
        .map(Book::try_from)
        .collect::<Result<Vec<_>, _>>()?;

    Ok(Json(ApiResponse { data: books }).into_response())
}

async fn get_book_by_id(
    State(app_state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let mut qb: sqlx::QueryBuilder<sqlx::Sqlite> = sqlx::QueryBuilder::new(BOOKS_BASE_QUERY);
    qb.push(" WHERE books.id = ").push_bind(id);

    let row = qb
        .build_query_as::<BookRow>()
        .fetch_optional(&app_state.db)
        .await?;

    match row {
        Some(row) => {
            let book = Book::try_from(row)?;
            Ok(Json(ApiResponse { data: book }).into_response())
        }
        None => Err(AppError::NotFound),
    }
}

async fn get_series(
    State(app_state): State<AppState>,
    Query(query): Query<Pagination>,
) -> Result<Response, AppError> {
    let recs = sqlx::query_as::<_, Series>(
        "SELECT id, name, sort FROM series ORDER BY sort LIMIT ? OFFSET ?",
    )
    .bind(query.limit())
    .bind(query.offset())
    .fetch_all(&app_state.db)
    .await?;

    Ok(Json(ApiResponse { data: recs }).into_response())
}

async fn get_series_by_id(
    State(app_state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let rec = sqlx::query_as::<_, Series>("SELECT id, name, sort FROM series WHERE id = ?")
        .bind(id)
        .fetch_optional(&app_state.db)
        .await?;

    match rec {
        Some(s) => Ok(Json(ApiResponse { data: s }).into_response()),
        None => Err(AppError::NotFound),
    }
}

async fn get_tags(
    State(app_state): State<AppState>,
    Query(query): Query<Pagination>,
) -> Result<Response, AppError> {
    let recs = sqlx::query_as::<_, Tag>(
        "SELECT id, name FROM tags ORDER BY name LIMIT ? OFFSET ?",
    )
    .bind(query.limit())
    .bind(query.offset())
    .fetch_all(&app_state.db)
    .await?;

    Ok(Json(ApiResponse { data: recs }).into_response())
}

async fn get_tag_by_id(
    State(app_state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let rec = sqlx::query_as::<_, Tag>("SELECT id, name FROM tags WHERE id = ?")
        .bind(id)
        .fetch_optional(&app_state.db)
        .await?;

    match rec {
        Some(tag) => Ok(Json(ApiResponse { data: tag }).into_response()),
        None => Err(AppError::NotFound),
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum ErrorKind {
    NotFound,
    InternalError,
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: ErrorKind,
}

#[derive(Debug)]
enum AppError {
    NotFound,
    Internal(anyhow::Error),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        match self {
            AppError::NotFound => (
                StatusCode::NOT_FOUND,
                Json(ErrorBody { error: ErrorKind::NotFound }),
            )
                .into_response(),
            AppError::Internal(e) => {
                tracing::error!("Internal error: {:?}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorBody { error: ErrorKind::InternalError }),
                )
                    .into_response()
            }
        }
    }
}

impl<E> From<E> for AppError
where
    E: Into<anyhow::Error>,
{
    fn from(err: E) -> Self {
        Self::Internal(err.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    async fn test_pool() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();

        sqlx::query(
            "CREATE TABLE authors (
                id   INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                sort TEXT,
                link TEXT NOT NULL DEFAULT ''
            )",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE books (
                id           INTEGER PRIMARY KEY,
                title        TEXT NOT NULL,
                pubdate      DATETIME,
                series_index REAL
            )",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE books_authors_link (
                id     INTEGER PRIMARY KEY,
                book   INTEGER NOT NULL,
                author INTEGER NOT NULL
            )",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE identifiers (
                id   INTEGER PRIMARY KEY,
                book INTEGER NOT NULL,
                type TEXT NOT NULL,
                val  TEXT NOT NULL
            )",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE series (
                id   INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                sort TEXT
            )",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE books_series_link (
                id     INTEGER PRIMARY KEY,
                book   INTEGER NOT NULL,
                series INTEGER NOT NULL
            )",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE tags (
                id   INTEGER PRIMARY KEY,
                name TEXT NOT NULL
            )",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE books_tags_link (
                id   INTEGER PRIMARY KEY,
                book INTEGER NOT NULL,
                tag  INTEGER NOT NULL
            )",
        )
        .execute(&pool)
        .await
        .unwrap();

        // Authors
        sqlx::query("INSERT INTO authors VALUES (1, 'Ursula K. Le Guin', 'Le Guin, Ursula K.', '')")
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO authors VALUES (2, 'Isaac Asimov', 'Asimov, Isaac', '')")
            .execute(&pool).await.unwrap();

        // Books: id=1 pubdate=1969, id=2 pubdate=1951
        // Alphabetically: Foundation < The Left Hand of Darkness
        sqlx::query("INSERT INTO books VALUES (1, 'The Left Hand of Darkness', '1969-03-01 00:00:00', 4.0)")
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO books VALUES (2, 'Foundation', '1951-01-01 00:00:00', 1.0)")
            .execute(&pool).await.unwrap();

        // Book 1 has two authors to test multi-author deduplication
        sqlx::query("INSERT INTO books_authors_link (book, author) VALUES (1, 1)")
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO books_authors_link (book, author) VALUES (1, 2)")
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO books_authors_link (book, author) VALUES (2, 2)")
            .execute(&pool).await.unwrap();

        // Series
        sqlx::query("INSERT INTO series VALUES (1, 'Hainish Cycle', 'Hainish Cycle')")
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO books_series_link (book, series) VALUES (1, 1)")
            .execute(&pool).await.unwrap();

        // Identifiers
        sqlx::query("INSERT INTO identifiers (book, type, val) VALUES (2, 'isbn', '9780553293357')")
            .execute(&pool).await.unwrap();

        // Tags
        sqlx::query("INSERT INTO tags VALUES (1, 'Fantasy')")
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO tags VALUES (2, 'Science Fiction')")
            .execute(&pool).await.unwrap();

        // Book 1: Fantasy + Science Fiction; Book 2: Science Fiction only
        sqlx::query("INSERT INTO books_tags_link (book, tag) VALUES (1, 1)")
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO books_tags_link (book, tag) VALUES (1, 2)")
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO books_tags_link (book, tag) VALUES (2, 2)")
            .execute(&pool).await.unwrap();

        pool
    }

    fn req(uri: &str) -> Request<Body> {
        Request::builder().uri(uri).body(Body::empty()).unwrap()
    }

    async fn json(body: Body) -> serde_json::Value {
        let bytes = to_bytes(body, usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn health_ok() {
        let app = build_app(test_pool().await);
        let resp = app.oneshot(req("/_health")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn authors_list() {
        let app = build_app(test_pool().await);
        let resp = app.oneshot(req("/v1/authors")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = json(resp.into_body()).await;
        assert_eq!(body["data"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn author_found() {
        let app = build_app(test_pool().await);
        let resp = app.oneshot(req("/v1/authors/1")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = json(resp.into_body()).await;
        assert_eq!(body["data"]["name"], "Ursula K. Le Guin");
    }

    #[tokio::test]
    async fn author_not_found() {
        let app = build_app(test_pool().await);
        let resp = app.oneshot(req("/v1/authors/99999")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        let body = json(resp.into_body()).await;
        assert_eq!(body["error"], "not_found");
    }

    #[tokio::test]
    async fn books_list_no_duplicates() {
        // Book 1 has 2 authors — should appear once, not twice
        let app = build_app(test_pool().await);
        let resp = app.oneshot(req("/v1/books")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = json(resp.into_body()).await;
        assert_eq!(body["data"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn book_has_multiple_authors() {
        let app = build_app(test_pool().await);
        let resp = app.oneshot(req("/v1/books/1")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = json(resp.into_body()).await;
        let authors = body["data"]["authors"].as_array().unwrap();
        assert_eq!(authors.len(), 2);
    }

    #[tokio::test]
    async fn book_has_tags() {
        let app = build_app(test_pool().await);
        let resp = app.oneshot(req("/v1/books/1")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = json(resp.into_body()).await;
        let tags = body["data"]["tags"].as_array().unwrap();
        assert_eq!(tags.len(), 2);
    }

    #[tokio::test]
    async fn books_filter_by_author() {
        let app = build_app(test_pool().await);
        // Author 1 (Le Guin) wrote only book 1
        let resp = app.oneshot(req("/v1/books?author_id=1")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = json(resp.into_body()).await;
        let data = body["data"].as_array().unwrap();
        assert_eq!(data.len(), 1);
        assert_eq!(data[0]["title"], "The Left Hand of Darkness");
    }

    #[tokio::test]
    async fn books_filter_by_series() {
        let app = build_app(test_pool().await);
        let resp = app.oneshot(req("/v1/books?series_id=1")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = json(resp.into_body()).await;
        let data = body["data"].as_array().unwrap();
        assert_eq!(data.len(), 1);
        assert_eq!(data[0]["title"], "The Left Hand of Darkness");
    }

    #[tokio::test]
    async fn books_filter_by_tag() {
        let app = build_app(test_pool().await);
        // tag 1 = Fantasy, only book 1 has it
        let resp = app.oneshot(req("/v1/books?tag_id=1")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = json(resp.into_body()).await;
        let data = body["data"].as_array().unwrap();
        assert_eq!(data.len(), 1);
        assert_eq!(data[0]["title"], "The Left Hand of Darkness");

        // tag 2 = Science Fiction, both books have it
        let app = build_app(test_pool().await);
        let resp = app.oneshot(req("/v1/books?tag_id=2")).await.unwrap();
        let body = json(resp.into_body()).await;
        assert_eq!(body["data"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn books_search_by_title() {
        let app = build_app(test_pool().await);
        let resp = app.oneshot(req("/v1/books?q=foundation")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = json(resp.into_body()).await;
        let data = body["data"].as_array().unwrap();
        assert_eq!(data.len(), 1);
        assert_eq!(data[0]["title"], "Foundation");
    }

    #[tokio::test]
    async fn books_search_by_author_name() {
        let app = build_app(test_pool().await);
        // "Le Guin" matches author 1, who wrote only book 1
        let resp = app.oneshot(req("/v1/books?q=Le+Guin")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = json(resp.into_body()).await;
        let data = body["data"].as_array().unwrap();
        assert_eq!(data.len(), 1);
        assert_eq!(data[0]["title"], "The Left Hand of Darkness");
    }

    #[tokio::test]
    async fn books_search_matches_both_title_and_author() {
        let app = build_app(test_pool().await);
        // "Asimov" matches author 2, who wrote both books
        let resp = app.oneshot(req("/v1/books?q=Asimov")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = json(resp.into_body()).await;
        assert_eq!(body["data"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn books_sort_by_title() {
        let app = build_app(test_pool().await);
        // Alphabetically: Foundation (id=2) < The Left Hand of Darkness (id=1)
        let resp = app.oneshot(req("/v1/books?sort=title")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = json(resp.into_body()).await;
        let data = body["data"].as_array().unwrap();
        assert_eq!(data[0]["title"], "Foundation");
        assert_eq!(data[1]["title"], "The Left Hand of Darkness");
    }

    #[tokio::test]
    async fn books_sort_by_title_desc() {
        let app = build_app(test_pool().await);
        let resp = app.oneshot(req("/v1/books?sort=title&sort_dir=desc")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = json(resp.into_body()).await;
        let data = body["data"].as_array().unwrap();
        assert_eq!(data[0]["title"], "The Left Hand of Darkness");
        assert_eq!(data[1]["title"], "Foundation");
    }

    #[tokio::test]
    async fn books_sort_by_pubdate() {
        let app = build_app(test_pool().await);
        // pubdate asc: Foundation 1951 (id=2) < Left Hand 1969 (id=1)
        let resp = app.oneshot(req("/v1/books?sort=pubdate")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = json(resp.into_body()).await;
        let data = body["data"].as_array().unwrap();
        assert_eq!(data[0]["title"], "Foundation");
        assert_eq!(data[1]["title"], "The Left Hand of Darkness");
    }

    #[tokio::test]
    async fn books_pagination() {
        let app = build_app(test_pool().await);
        let resp = app.oneshot(req("/v1/books?limit=1&offset=0")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = json(resp.into_body()).await;
        assert_eq!(body["data"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn book_found() {
        let app = build_app(test_pool().await);
        let resp = app.oneshot(req("/v1/books/1")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = json(resp.into_body()).await;
        assert_eq!(body["data"]["title"], "The Left Hand of Darkness");
    }

    #[tokio::test]
    async fn book_not_found() {
        let app = build_app(test_pool().await);
        let resp = app.oneshot(req("/v1/books/99999")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        let body = json(resp.into_body()).await;
        assert_eq!(body["error"], "not_found");
    }

    #[tokio::test]
    async fn series_list() {
        let app = build_app(test_pool().await);
        let resp = app.oneshot(req("/v1/series")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = json(resp.into_body()).await;
        assert_eq!(body["data"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn series_found() {
        let app = build_app(test_pool().await);
        let resp = app.oneshot(req("/v1/series/1")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = json(resp.into_body()).await;
        assert_eq!(body["data"]["name"], "Hainish Cycle");
    }

    #[tokio::test]
    async fn series_not_found() {
        let app = build_app(test_pool().await);
        let resp = app.oneshot(req("/v1/series/99999")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        let body = json(resp.into_body()).await;
        assert_eq!(body["error"], "not_found");
    }

    #[tokio::test]
    async fn tags_list() {
        let app = build_app(test_pool().await);
        let resp = app.oneshot(req("/v1/tags")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = json(resp.into_body()).await;
        // Tags are ordered by name: Fantasy, Science Fiction
        let data = body["data"].as_array().unwrap();
        assert_eq!(data.len(), 2);
        assert_eq!(data[0]["name"], "Fantasy");
    }

    #[tokio::test]
    async fn tag_found() {
        let app = build_app(test_pool().await);
        let resp = app.oneshot(req("/v1/tags/1")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = json(resp.into_body()).await;
        assert_eq!(body["data"]["name"], "Fantasy");
    }

    #[tokio::test]
    async fn tag_not_found() {
        let app = build_app(test_pool().await);
        let resp = app.oneshot(req("/v1/tags/99999")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        let body = json(resp.into_body()).await;
        assert_eq!(body["error"], "not_found");
    }

    #[tokio::test]
    async fn pagination_cap() {
        let app = build_app(test_pool().await);
        let resp = app.oneshot(req("/v1/books?limit=999999")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = json(resp.into_body()).await;
        assert_eq!(body["data"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn books_ordered_consistently() {
        let pool = test_pool().await;
        let app1 = build_app(pool.clone());
        let app2 = build_app(pool);
        let body1 = json(app1.oneshot(req("/v1/books")).await.unwrap().into_body()).await;
        let body2 = json(app2.oneshot(req("/v1/books")).await.unwrap().into_body()).await;
        assert_eq!(body1["data"][0]["id"], body2["data"][0]["id"]);
    }
}