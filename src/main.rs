use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    sync::{Arc, RwLock},
};
use tower_http::cors::{Any, CorsLayer};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TodoKind {
    PR,
    Issue,
    WIP,
    Task,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TodoStatus {
    Open,
    InProgress,
    Merged,
    Done,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoItem {
    pub id: String,
    pub title: String,
    pub kind: TodoKind,
    pub status: TodoStatus,
    pub repo: Option<String>,
    pub score: Option<f64>,
    pub url: Option<String>,
    pub metadata: HashMap<String, String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateTodoRequest {
    pub title: String,
    pub kind: Option<TodoKind>,
    pub repo: Option<String>,
    pub score: Option<f64>,
    pub url: Option<String>,
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateTodoRequest {
    pub title: Option<String>,
    pub status: Option<TodoStatus>,
    pub score: Option<f64>,
    pub metadata: Option<HashMap<String, String>>,
}

#[derive(Clone)]
pub struct AppState {
    pub db_path: PathBuf,
    pub todos: Arc<RwLock<HashMap<String, TodoItem>>>,
}

impl AppState {
    pub fn new(db_path: PathBuf) -> Self {
        let mut todos = HashMap::new();
        if db_path.exists() {
            if let Ok(content) = fs::read_to_string(&db_path) {
                if let Ok(loaded) = serde_json::from_str::<HashMap<String, TodoItem>>(&content) {
                    todos = loaded;
                }
            }
        }
        Self {
            db_path,
            todos: Arc::new(RwLock::new(todos)),
        }
    }

    pub fn persist(&self) {
        if let Ok(guard) = self.todos.read() {
            if let Ok(serialized) = serde_json::to_string_pretty(&*guard) {
                let _ = fs::write(&self.db_path, serialized);
            }
        }
    }
}

// REST Handlers
async fn list_todos(State(state): State<AppState>) -> impl IntoResponse {
    let guard = state.todos.read().unwrap();
    let mut list: Vec<TodoItem> = guard.values().cloned().collect();
    // Sort by score descending or updated_at descending
    list.sort_by(|a, b| {
        b.score
            .unwrap_or(0.0)
            .partial_cmp(&a.score.unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Json(list)
}

async fn get_todo(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<TodoItem>, StatusCode> {
    let guard = state.todos.read().unwrap();
    guard
        .get(&id)
        .cloned()
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn create_todo(
    State(state): State<AppState>,
    Json(payload): Json<CreateTodoRequest>,
) -> impl IntoResponse {
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let item = TodoItem {
        id: id.clone(),
        title: payload.title,
        kind: payload.kind.unwrap_or(TodoKind::Task),
        status: TodoStatus::Open,
        repo: payload.repo,
        score: payload.score,
        url: payload.url,
        metadata: payload.metadata,
        created_at: now.clone(),
        updated_at: now,
    };

    {
        let mut guard = state.todos.write().unwrap();
        guard.insert(id, item.clone());
    }
    state.persist();
    (StatusCode::CREATED, Json(item))
}

async fn update_todo(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(payload): Json<UpdateTodoRequest>,
) -> Result<Json<TodoItem>, StatusCode> {
    let now = chrono::Utc::now().to_rfc3339();
    let mut guard = state.todos.write().unwrap();
    if let Some(item) = guard.get_mut(&id) {
        if let Some(title) = payload.title {
            item.title = title;
        }
        if let Some(status) = payload.status {
            item.status = status;
        }
        if let Some(score) = payload.score {
            item.score = Some(score);
        }
        if let Some(meta) = payload.metadata {
            item.metadata.extend(meta);
        }
        item.updated_at = now;
        let cloned = item.clone();
        drop(guard);
        state.persist();
        Ok(Json(cloned))
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

async fn health_check() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "online",
        "service": "omni-todos",
        "version": "0.1.0",
        "peer": "todos.vaked.dev"
    }))
}

// MCP JSON-RPC Stdio Loop
pub async fn run_mcp(state: AppState) {
    use std::io::{self, BufRead};
    let stdin = io::stdin();
    for line_str in stdin.lock().lines().map_while(Result::ok) {
        if line_str.trim().is_empty() {
            continue;
        }
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&line_str) {
            let id = val.get("id").cloned();
            let method = val.get("method").and_then(|m| m.as_str()).unwrap_or("");

            match method {
                "tools/list" => {
                    let resp = serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "tools": [
                                {
                                    "name": "omni_todos_list",
                                    "description": "List all sovereign todos from omni-todos / todos.vaked.dev",
                                    "inputSchema": {
                                        "type": "object",
                                        "properties": {
                                            "status": { "type": "string", "enum": ["Open", "InProgress", "Merged", "Done", "Skipped"] }
                                        }
                                    }
                                },
                                {
                                    "name": "omni_todos_create",
                                    "description": "Create a new sovereign todo item",
                                    "inputSchema": {
                                        "type": "object",
                                        "properties": {
                                            "title": { "type": "string" },
                                            "repo": { "type": "string" },
                                            "kind": { "type": "string", "enum": ["PR", "Issue", "WIP", "Task"] },
                                            "score": { "type": "number" },
                                            "url": { "type": "string" }
                                        },
                                        "required": ["title"]
                                    }
                                },
                                {
                                    "name": "omni_todos_resolve",
                                    "description": "Mark a todo item as Merged or Done",
                                    "inputSchema": {
                                        "type": "object",
                                        "properties": {
                                            "id": { "type": "string" },
                                            "status": { "type": "string", "enum": ["Merged", "Done", "Skipped"] }
                                        },
                                        "required": ["id"]
                                    }
                                }
                            ]
                        }
                    });
                    println!("{}", resp);
                }
                "tools/call" => {
                    let params = val
                        .get("params")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null);
                    let tool_name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
                    let args = params
                        .get("arguments")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null);

                    let result = match tool_name {
                        "omni_todos_list" => {
                            let guard = state.todos.read().unwrap();
                            let list: Vec<TodoItem> = guard.values().cloned().collect();
                            serde_json::json!({
                                "content": [{
                                    "type": "text",
                                    "text": serde_json::to_string_pretty(&list).unwrap_or_default()
                                }]
                            })
                        }
                        "omni_todos_create" => {
                            let title = args
                                .get("title")
                                .and_then(|t| t.as_str())
                                .unwrap_or("Untitled")
                                .to_string();
                            let repo = args.get("repo").and_then(|r| r.as_str()).map(String::from);
                            let score = args.get("score").and_then(|s| s.as_f64());
                            let url = args.get("url").and_then(|u| u.as_str()).map(String::from);
                            let now = chrono::Utc::now().to_rfc3339();
                            let new_id = uuid::Uuid::new_v4().to_string();

                            let item = TodoItem {
                                id: new_id.clone(),
                                title,
                                kind: TodoKind::Task,
                                status: TodoStatus::Open,
                                repo,
                                score,
                                url,
                                metadata: HashMap::new(),
                                created_at: now.clone(),
                                updated_at: now,
                            };
                            {
                                let mut guard = state.todos.write().unwrap();
                                guard.insert(new_id, item.clone());
                            }
                            state.persist();
                            serde_json::json!({
                                "content": [{
                                    "type": "text",
                                    "text": format!("Created todo: {}", item.id)
                                }]
                            })
                        }
                        "omni_todos_resolve" => {
                            let target_id = args.get("id").and_then(|i| i.as_str()).unwrap_or("");
                            let target_status = args
                                .get("status")
                                .and_then(|s| s.as_str())
                                .unwrap_or("Done");
                            let mut guard = state.todos.write().unwrap();
                            if let Some(item) = guard.get_mut(target_id) {
                                item.status = match target_status {
                                    "Merged" => TodoStatus::Merged,
                                    "Skipped" => TodoStatus::Skipped,
                                    _ => TodoStatus::Done,
                                };
                                item.updated_at = chrono::Utc::now().to_rfc3339();
                                drop(guard);
                                state.persist();
                                serde_json::json!({
                                    "content": [{
                                        "type": "text",
                                        "text": format!("Todo {} marked as {}", target_id, target_status)
                                    }]
                                })
                            } else {
                                serde_json::json!({
                                    "isError": true,
                                    "content": [{ "type": "text", "text": "Todo ID not found" }]
                                })
                            }
                        }
                        _ => {
                            serde_json::json!({
                                "isError": true,
                                "content": [{ "type": "text", "text": format!("Unknown tool: {}", tool_name) }]
                            })
                        }
                    };

                    let resp = serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    });
                    println!("{}", resp);
                }
                _ => {
                    let resp = serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {}
                    });
                    println!("{}", resp);
                }
            }
        }
    }
}

#[derive(Parser, Debug)]
#[command(name = "omni-todos")]
#[command(about = "Sovereign Ultra-Evolved TODO Engine & todos.vaked.dev API", long_about = None)]
struct Cli {
    #[arg(short, long, default_value = "todos.json")]
    db: PathBuf,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Start the REST HTTP API server
    Serve {
        #[arg(short, long, default_value = "127.0.0.1:8099")]
        bind: String,
    },
    /// Run as an MCP Server over stdio
    Mcp,
    /// List all todos
    List,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let state = AppState::new(cli.db);

    match cli.command.unwrap_or(Commands::Serve {
        bind: "127.0.0.1:8099".to_string(),
    }) {
        Commands::Serve { bind } => {
            let app = Router::new()
                .route("/health", get(health_check))
                .route("/api/todos", get(list_todos).post(create_todo))
                .route("/api/todos/{id}", get(get_todo).patch(update_todo))
                .layer(
                    CorsLayer::new()
                        .allow_origin(Any)
                        .allow_methods(Any)
                        .allow_headers(Any),
                )
                .with_state(state);

            println!(
                "[*] omni-todos HTTP API server listening on http://{}",
                bind
            );
            let listener = tokio::net::TcpListener::bind(&bind).await?;
            axum::serve(listener, app).await?;
        }
        Commands::Mcp => {
            run_mcp(state).await;
        }
        Commands::List => {
            let guard = state.todos.read().unwrap();
            let mut list: Vec<TodoItem> = guard.values().cloned().collect();
            list.sort_by(|a, b| {
                b.score
                    .unwrap_or(0.0)
                    .partial_cmp(&a.score.unwrap_or(0.0))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            println!(
                "{:<38} {:<10} {:<10} {:<6} TITLE",
                "ID", "KIND", "STATUS", "SCORE"
            );
            println!("{}", "-".repeat(90));
            for item in list {
                println!(
                    "{:<38} {:<10?} {:<10?} {:<6.2} {}",
                    item.id,
                    item.kind,
                    item.status,
                    item.score.unwrap_or(0.0),
                    item.title
                );
            }
        }
    }

    Ok(())
}
