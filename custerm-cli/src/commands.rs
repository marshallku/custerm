use clap::{Parser, Subcommand};
use serde_json::json;

#[derive(Parser)]
#[command(name = "custerm", about = "custerm CLI")]
pub struct Cli {
    /// Socket path override
    #[arg(long)]
    pub socket: Option<String>,

    /// Output JSON format
    #[arg(long, default_value_t = false)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Ping the running custerm instance
    Ping,

    /// Window management
    #[command(subcommand)]
    Window(WindowCommand),

    /// Workspace management
    #[command(subcommand)]
    Workspace(WorkspaceCommand),

    /// Session/surface management
    #[command(subcommand)]
    Session(SessionCommand),

    /// Background image management
    #[command(subcommand)]
    Background(BackgroundCommand),
}

#[derive(Subcommand)]
pub enum WindowCommand {
    /// List all windows
    List,
    /// Create a new window
    New,
    /// Focus a window
    Focus { id: String },
    /// Close a window
    Close { id: Option<String> },
}

#[derive(Subcommand)]
pub enum WorkspaceCommand {
    /// List workspaces
    List,
    /// Create a new workspace
    New {
        #[arg(long)]
        name: Option<String>,
    },
    /// Select/switch to a workspace
    Select { id: String },
    /// Close a workspace
    Close { id: Option<String> },
    /// Rename a workspace
    Rename { id: String, name: String },
}

#[derive(Subcommand)]
pub enum SessionCommand {
    /// List sessions
    List,
    /// Send text to a session
    Send {
        #[arg(long)]
        id: String,
        text: String,
    },
    /// Read screen content
    Read {
        #[arg(long)]
        id: Option<String>,
        #[arg(long, default_value_t = 50)]
        lines: u32,
    },
    /// Close a session
    Close { id: String },
}

#[derive(Subcommand)]
pub enum BackgroundCommand {
    /// Set background image
    Set { path: String },
    /// Switch to next random background
    Next,
    /// Toggle background visibility
    Toggle,
}

impl Cli {
    pub fn method(&self) -> String {
        match &self.command {
            Command::Ping => "system.ping".to_string(),
            Command::Window(cmd) => match cmd {
                WindowCommand::List => "window.list",
                WindowCommand::New => "window.create",
                WindowCommand::Focus { .. } => "window.focus",
                WindowCommand::Close { .. } => "window.close",
            }
            .to_string(),
            Command::Workspace(cmd) => match cmd {
                WorkspaceCommand::List => "workspace.list",
                WorkspaceCommand::New { .. } => "workspace.create",
                WorkspaceCommand::Select { .. } => "workspace.select",
                WorkspaceCommand::Close { .. } => "workspace.close",
                WorkspaceCommand::Rename { .. } => "workspace.rename",
            }
            .to_string(),
            Command::Session(cmd) => match cmd {
                SessionCommand::List => "session.list",
                SessionCommand::Send { .. } => "session.send_text",
                SessionCommand::Read { .. } => "session.read_text",
                SessionCommand::Close { .. } => "session.close",
            }
            .to_string(),
            Command::Background(cmd) => match cmd {
                BackgroundCommand::Set { .. } => "background.set",
                BackgroundCommand::Next => "background.next",
                BackgroundCommand::Toggle => "background.toggle",
            }
            .to_string(),
        }
    }

    pub fn params(&self) -> serde_json::Value {
        match &self.command {
            Command::Ping => json!({}),
            Command::Window(cmd) => match cmd {
                WindowCommand::List | WindowCommand::New => json!({}),
                WindowCommand::Focus { id } => json!({ "window_id": id }),
                WindowCommand::Close { id } => json!({ "window_id": id }),
            },
            Command::Workspace(cmd) => match cmd {
                WorkspaceCommand::List => json!({}),
                WorkspaceCommand::New { name } => json!({ "name": name }),
                WorkspaceCommand::Select { id } => json!({ "workspace_id": id }),
                WorkspaceCommand::Close { id } => json!({ "workspace_id": id }),
                WorkspaceCommand::Rename { id, name } => {
                    json!({ "workspace_id": id, "name": name })
                }
            },
            Command::Session(cmd) => match cmd {
                SessionCommand::List => json!({}),
                SessionCommand::Send { id, text } => json!({ "session_id": id, "text": text }),
                SessionCommand::Read { id, lines } => json!({ "session_id": id, "lines": lines }),
                SessionCommand::Close { id } => json!({ "session_id": id }),
            },
            Command::Background(cmd) => match cmd {
                BackgroundCommand::Set { path } => json!({ "path": path }),
                BackgroundCommand::Next | BackgroundCommand::Toggle => json!({}),
            },
        }
    }
}
