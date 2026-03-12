# calibre-metadata-api

A read-only REST API for querying a [Calibre](https://calibre-ebook.com/) ebook library database over HTTP. Point it at a `metadata.db` file and get books, authors, series, and tags as JSON.

## Setup

Copy your Calibre `metadata.db` to the project root, then create a `cma.toml` config file:

```toml
database_url = "metadata.db"
```

## Running

```sh
cargo run
# or, after building:
./target/release/cma
```

The server listens on `127.0.0.1:3002` by default.

### Options

| Flag | Default | Description |
|---|---|---|
| `--bind-addr` | `127.0.0.1:3002` | Address and port to listen on |
| `--config-file` | `cma.toml` | Path to config file |
| `--log-level` | `DEBUG` | Log verbosity (`TRACE`, `DEBUG`, `INFO`, `WARN`, `ERROR`) |

## API

All responses have the shape `{ "data": [...] }` for lists or `{ "data": {...} }` for single items.

Errors return JSON with an `error` field:

```json
{ "error": "not_found" }
{ "error": "internal_error" }
```

CORS is enabled for all origins, so the API can be called directly from browser-based clients.

List endpoints accept `?limit=N&offset=N` for pagination (default limit: 100, max: 1000).

### Endpoints

#### `GET /_health`
Returns `200 OK` if the server and database are healthy, `503` if the database is unreachable.

#### `GET /v1/authors`

Returns all authors, ordered by sort name. Supports `?limit=` and `?offset=`.

```json
{
  "data": [
    { "id": 1, "name": "Ursula K. Le Guin", "sort": "Le Guin, Ursula K.", "link": "" }
  ]
}
```

#### `GET /v1/authors/{id}`

Returns a single author by ID, or `404` if not found.

#### `GET /v1/books`

Returns books. Supports filtering, sorting, and pagination:

| Parameter | Description |
|---|---|
| `author_id` | Only books by this author ID |
| `series_id` | Only books in this series ID |
| `tag_id` | Only books with this tag ID |
| `q` | Search by title or author name (case-insensitive) |
| `sort` | Sort field: `id` (default), `title`, `pubdate` |
| `sort_dir` | Sort direction: `asc` (default), `desc` |
| `limit` | Max results (default: 100, max: 1000) |
| `offset` | Skip N results (default: 0) |

```sh
# Books by author 3, page 2
GET /v1/books?author_id=3&limit=20&offset=20

# Books with a specific tag
GET /v1/books?tag_id=7

# Search by title or author name
GET /v1/books?q=asimov

# Sorted alphabetically, newest first
GET /v1/books?sort=title&sort_dir=desc
```

```json
{
  "data": [
    {
      "id": 42,
      "title": "The Left Hand of Darkness",
      "pubdate": "1969-03-01T00:00:00",
      "authors": [{ "id": 1, "name": "Ursula K. Le Guin" }],
      "tags": ["Science Fiction", "Fantasy"],
      "isbn": "9780441478125",
      "series_name": "Hainish Cycle",
      "series_index": 4.0
    }
  ]
}
```

#### `GET /v1/books/{id}`

Returns a single book by ID, or `404` if not found.

#### `GET /v1/series`

Returns all series, ordered by sort name. Supports `?limit=` and `?offset=`.

#### `GET /v1/series/{id}`

Returns a single series by ID, or `404` if not found.

#### `GET /v1/tags`

Returns all tags, ordered by name. Supports `?limit=` and `?offset=`.

#### `GET /v1/tags/{id}`

Returns a single tag by ID, or `404` if not found.

## Dev

```sh
# Copy your Calibre library database
cp ~/path/to/Calibre\ Library/metadata.db .

# Run with auto-reload (requires cargo-watch)
cargo watch -x run

# Run tests (no database file needed — tests use an in-memory SQLite fixture)
cargo test
```