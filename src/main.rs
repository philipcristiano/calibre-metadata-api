use clap::Parser;
use serde::{Deserialize, Serialize};

use sqlx::sqlite::SqlitePool;

use axum::{
    Json, Router,
    extract::{FromRef, Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use axum_extra::extract::Query;
use std::net::SocketAddr;

use tower_cookies::CookieManagerLayer;

#[derive(Parser, Debug)]
pub struct Args {
    #[arg(short, long, default_value = "127.0.0.1:3002")]
    bind_addr: String,
    #[arg(short, long, default_value = "cma.toml")]
    config_file: String,
    #[arg(short, long, value_enum, default_value = "DEBUG")]
    log_level: tracing::Level,
    #[arg(long, action)]
    log_json: bool,
}

#[derive(Clone, Debug, Deserialize)]
struct AppConfig {
    database_url: String,
}

#[derive(FromRef, Clone, Debug)]
struct AppState {
    db: SqlitePool,
}

impl AppState {
    fn from_config(_config: AppConfig, db: SqlitePool) -> Self {
        AppState { db }
    }
}

fn read_app_config(path: &str) -> anyhow::Result<AppConfig> {
    use std::fs;
    let contents = fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("Could not read config file {}: {}", path, e))?;
    let config = toml::from_str(&contents)
        .map_err(|e| anyhow::anyhow!("Problems parsing config file: {}", e))?;
    Ok(config)
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

    let app_state = AppState::from_config(app_config, pool);

    let app = Router::new()
        .route("/", get(root))
        .route("/_health", get(health))
        .route("/v1/authors", get(get_authors))
        .route("/v1/authors/{id}", get(get_author))
        .route("/v1/books", get(get_books))
        .route("/v1/books/{id}", get(get_book))
        .route("/v1/series", get(get_series))
        .route("/v1/tags", get(get_tags))
        .with_state(app_state)
        .layer(CookieManagerLayer::new())
        .layer(tower_http::compression::CompressionLayer::new())
        .layer(service_conventions::tracing_http::trace_layer(
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

async fn root() -> Response {
    "Hello World".into_response()
}

#[derive(Debug, Deserialize, Default)]
struct ListQuery {
    limit: Option<i64>,
    offset: Option<i64>,
}

impl ListQuery {
    fn limit(&self) -> i64 {
        self.limit.unwrap_or(100)
    }
    fn offset(&self) -> i64 {
        self.offset.unwrap_or(0)
    }
}

#[derive(Debug, Deserialize, Default)]
struct BooksQuery {
    author_id: Option<i64>,
    series_id: Option<i64>,
    q: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
}

impl BooksQuery {
    fn limit(&self) -> i64 {
        self.limit.unwrap_or(100)
    }
    fn offset(&self) -> i64 {
        self.offset.unwrap_or(0)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct V1APIResponse {
    data: Vec<CDBStruct>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct V1ItemResponse {
    data: CDBStruct,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum CDBStruct {
    Author(Author),
    Book(Book),
    Series(Series),
    Tag(Tag),
}

#[derive(Debug, Clone, Deserialize, Serialize, sqlx::FromRow)]
struct Author {
    id: i64,
    name: String,
    sort: Option<String>,
    link: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, sqlx::FromRow)]
struct Book {
    id: i64,
    title: String,
    pubdate: Option<chrono::NaiveDateTime>,
    author_name: String,
    author_id: i64,
    isbn: Option<String>,
    series_name: Option<String>,
    series_index: Option<f64>,
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
    Query(query): Query<ListQuery>,
) -> Result<Response, AppError> {
    let recs = sqlx::query_as::<_, Author>(
        "SELECT id, name, sort, link FROM authors ORDER BY sort LIMIT ? OFFSET ?",
    )
    .bind(query.limit())
    .bind(query.offset())
    .fetch_all(&app_state.db)
    .await?;

    let data = recs.into_iter().map(CDBStruct::Author).collect();
    Ok(Json(V1APIResponse { data }).into_response())
}

async fn get_author(
    State(app_state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let rec =
        sqlx::query_as::<_, Author>("SELECT id, name, sort, link FROM authors WHERE id = ?")
            .bind(id)
            .fetch_optional(&app_state.db)
            .await?;

    match rec {
        Some(author) => Ok(Json(V1ItemResponse {
            data: CDBStruct::Author(author),
        })
        .into_response()),
        None => Ok(StatusCode::NOT_FOUND.into_response()),
    }
}

const BOOKS_BASE_QUERY: &str = "
    SELECT books.id as id, title, pubdate,
           authors.name as author_name, authors.id as author_id,
           i.val as isbn,
           s.name as series_name,
           books.series_index as series_index
    FROM books
    JOIN books_authors_link bal ON bal.book = books.id
    JOIN authors ON bal.author = authors.id
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
        qb.push("authors.id = ").push_bind(author_id);
        has_where = true;
    }

    if let Some(series_id) = query.series_id {
        qb.push(if has_where { " AND " } else { " WHERE " });
        qb.push("bsl.series = ").push_bind(series_id);
        has_where = true;
    }

    if let Some(ref q) = query.q {
        qb.push(if has_where { " AND " } else { " WHERE " });
        qb.push("books.title LIKE '%' || ").push_bind(q).push(" || '%'");
    }

    qb.push(" LIMIT ").push_bind(query.limit());
    qb.push(" OFFSET ").push_bind(query.offset());

    let recs = qb
        .build_query_as::<Book>()
        .fetch_all(&app_state.db)
        .await?;

    let data = recs.into_iter().map(CDBStruct::Book).collect();
    Ok(Json(V1APIResponse { data }).into_response())
}

async fn get_book(
    State(app_state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let mut qb: sqlx::QueryBuilder<sqlx::Sqlite> = sqlx::QueryBuilder::new(BOOKS_BASE_QUERY);
    qb.push(" WHERE books.id = ").push_bind(id);

    let rec = qb
        .build_query_as::<Book>()
        .fetch_optional(&app_state.db)
        .await?;

    match rec {
        Some(book) => Ok(Json(V1ItemResponse {
            data: CDBStruct::Book(book),
        })
        .into_response()),
        None => Ok(StatusCode::NOT_FOUND.into_response()),
    }
}

async fn get_series(
    State(app_state): State<AppState>,
    Query(query): Query<ListQuery>,
) -> Result<Response, AppError> {
    let recs = sqlx::query_as::<_, Series>(
        "SELECT id, name, sort FROM series ORDER BY sort LIMIT ? OFFSET ?",
    )
    .bind(query.limit())
    .bind(query.offset())
    .fetch_all(&app_state.db)
    .await?;

    let data = recs.into_iter().map(CDBStruct::Series).collect();
    Ok(Json(V1APIResponse { data }).into_response())
}

async fn get_tags(
    State(app_state): State<AppState>,
    Query(query): Query<ListQuery>,
) -> Result<Response, AppError> {
    let recs = sqlx::query_as::<_, Tag>(
        "SELECT id, name FROM tags ORDER BY name LIMIT ? OFFSET ?",
    )
    .bind(query.limit())
    .bind(query.offset())
    .fetch_all(&app_state.db)
    .await?;

    let data = recs.into_iter().map(CDBStruct::Tag).collect();
    Ok(Json(V1APIResponse { data }).into_response())
}

// Make our own error that wraps `anyhow::Error`.
#[derive(Debug)]
struct AppError(anyhow::Error);

// Tell axum how to convert `AppError` into a response.
impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        tracing::error!("HTTP Error {:?}", &self);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Something went wrong: {}", self.0),
        )
            .into_response()
    }
}

// This enables using `?` on functions that return `Result<_, anyhow::Error>` to turn them into
// `Result<_, AppError>`. That way you don't need to do that manually.
impl<E> From<E> for AppError
where
    E: Into<anyhow::Error>,
{
    fn from(err: E) -> Self {
        Self(err.into())
    }
}
