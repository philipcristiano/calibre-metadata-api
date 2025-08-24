use clap::Parser;
use serde::{Deserialize, Serialize};

use sqlx::sqlite::SqlitePool;

use axum::{
    Form, Json, Router,
    extract::{FromRef, Path, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
    routing::{delete, get, post},
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
    fn from_config(item: AppConfig, db: SqlitePool) -> Self {
        AppState { db }
    }
}

fn read_app_config(path: String) -> AppConfig {
    use std::fs;
    let config_file_error_msg = format!("Could not read config file {}", path);
    let config_file_contents = fs::read_to_string(path).expect(&config_file_error_msg);
    let app_config: AppConfig =
        toml::from_str(&config_file_contents).expect("Problems parsing config file");

    app_config
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    service_conventions::tracing::setup(args.log_level);

    let app_config = read_app_config(args.config_file);

    // start by making a database connection.
    tracing::info!("connecting to database");
    let pool = SqlitePool::connect(&app_config.database_url)
        .await
        .expect("cannot connect to db");

    let app_state = AppState::from_config(app_config, pool);

    let app = Router::new()
        // `get /` goes to `root`
        .route("/", get(root))
        .route("/v1/authors", get(get_author))
        //.route("/v1/authors/{author_id}", get(get_authors))
        .with_state(app_state.clone())
        .layer(CookieManagerLayer::new())
        .layer(tower_http::compression::CompressionLayer::new())
        .layer(service_conventions::tracing_http::trace_layer(
            tracing::Level::INFO,
        ))
        .route("/_health", get(health));

    let addr: SocketAddr = args.bind_addr.parse().expect("Expected bind addr");
    tracing::info!("listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn health() -> Response {
    "OK".into_response()
}

async fn root(State(app_state): State<AppState>) -> Result<Response, AppError> {
    Ok("Hello World".into_response())
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct V1APIResponse {
    data: Vec<CDBStruct>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum CDBStruct {
    Author(Author),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct Author {
    id: i64,
    name: String,
    sort: Option<String>,
    link: String,
}

async fn get_author(State(app_state): State<AppState>) -> Result<Response, AppError> {
    let recs = sqlx::query_as!(
        Author,
        r#"
            SELECT id, name, sort, link
            FROM authors
        "#
    )
    .fetch_all(&app_state.db)
    .await?;

    let cdbstruct = recs.into_iter().map(CDBStruct::Author).collect();
    let resp = V1APIResponse { data: cdbstruct };
    Ok(Json(resp).into_response())
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
