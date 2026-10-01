use clap::{Args, Parser, Subcommand};
use macrun::{
    model::{Reply, Request},
    wire,
};
use serde_json::{Value, json};
use std::path::PathBuf;
#[derive(Parser)]
#[command(version, about = "Remote macOS build and window automation demo")]
struct Cli {
    #[arg(
        long,
        global = true,
        env = "MACRUN_SOCKET",
        default_value = "/tmp/macrun.sock"
    )]
    socket: PathBuf,
    #[arg(long, global = true, default_value = ".")]
    workspace: PathBuf,
    #[command(subcommand)]
    command: Cmd,
}
#[derive(Subcommand)]
enum Cmd {
    /// Generate a local TLS identity and shared token (copy cert.der and token to worker).
    Init {
        #[arg(long)]
        data: PathBuf,
    },
    Serve {
        #[arg(long, default_value = "0.0.0.0:7443")]
        listen: std::net::SocketAddr,
        #[arg(long)]
        data: PathBuf,
    },
    Worker {
        #[arg(long)]
        server: std::net::SocketAddr,
        #[arg(long)]
        cert: PathBuf,
        #[arg(long)]
        token_file: PathBuf,
        #[arg(long)]
        data: PathBuf,
        #[arg(long, default_value = "cua-driver")]
        cua_binary: String,
        #[arg(long)]
        cua_socket: Option<String>,
    },
    Status {
        #[arg(long)]
        json: bool,
    },
    Sync,
    Build,
    Test {
        #[arg(long)]
        filter: Option<String>,
    },
    Run,
    Shot(Target),
    Stop {
        #[arg(long)]
        run: Option<String>,
    },
    Cancel {
        job_id: String,
    },
    Ui {
        #[command(subcommand)]
        action: Ui,
    },
}
#[derive(Args)]
struct Target {
    #[arg(long)]
    run: Option<String>,
    #[arg(long)]
    window: Option<u64>,
}
#[derive(Args)]
struct ActionTarget {
    #[arg(long)]
    run: String,
    #[arg(long)]
    window: u64,
    #[arg(long)]
    snapshot: String,
}
#[derive(Subcommand)]
enum Ui {
    Snapshot(Target),
    Click {
        #[command(flatten)]
        target: ActionTarget,
        #[arg(long, conflicts_with = "x")]
        element: Option<String>,
        #[arg(long, requires = "y", allow_hyphen_values = true)]
        x: Option<f64>,
        #[arg(long, requires = "x", allow_hyphen_values = true)]
        y: Option<f64>,
    },
    Type {
        #[command(flatten)]
        target: ActionTarget,
        #[arg(long)]
        element: String,
        #[arg(long)]
        text: String,
    },
    Key {
        #[command(flatten)]
        target: ActionTarget,
        #[arg(long)]
        key: String,
    },
    /// dy is signed wheel steps (positive down), not pixels/points.
    Scroll {
        #[command(flatten)]
        target: ActionTarget,
        #[arg(long)]
        element: String,
        #[arg(long, allow_hyphen_values = true)]
        dy: i64,
    },
}
fn target(t: ActionTarget) -> Value {
    json!({"run":t.run,"window":t.window,"snapshot":t.snapshot})
}
#[tokio::main]
async fn main() {
    if let Err(e) = entry().await {
        eprintln!("{e:#}");
        std::process::exit(13);
    }
}
async fn entry() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let (kind, args) = match cli.command {
        Cmd::Init { data } => {
            wire::init_tls(&data)?;
            println!(
                "Created identity in {}. Copy cert.der and token to the worker; keep key.der on the server.",
                data.display()
            );
            return Ok(());
        }
        Cmd::Serve { listen, data } => {
            return macrun::server::serve(listen, cli.socket, data).await;
        }
        Cmd::Worker {
            server,
            cert,
            token_file,
            data,
            cua_binary,
            cua_socket,
        } => {
            return macrun::worker::worker(macrun::worker::Options {
                server,
                cert,
                token_file,
                data,
                cua_binary,
                cua_socket,
            })
            .await;
        }
        Cmd::Status { .. } => ("status", json!({})),
        Cmd::Sync => ("sync", json!({})),
        Cmd::Build => ("build", json!({})),
        Cmd::Test { filter } => ("test", json!({"filter":filter})),
        Cmd::Run => ("run", json!({})),
        Cmd::Shot(t) => ("shot", json!({"run":t.run,"window":t.window})),
        Cmd::Stop { run } => ("stop", json!({"run":run})),
        Cmd::Cancel { job_id } => ("cancel", json!({"job_id":job_id})),
        Cmd::Ui { action } => match action {
            Ui::Snapshot(t) => ("snapshot", json!({"run":t.run,"window":t.window})),
            Ui::Click {
                target: t,
                element,
                x,
                y,
            } => {
                if element.is_none() && (x.is_none() || y.is_none()) {
                    eprintln!("click requires --element or both --x and --y");
                    std::process::exit(2);
                }
                let mut a = target(t);
                if let Some(e) = element {
                    a["element"] = json!(e);
                }
                if let Some(x) = x {
                    a["x"] = json!(x);
                    a["y"] = json!(y);
                }
                ("click", a)
            }
            Ui::Type {
                target: t,
                element,
                text,
            } => {
                let mut a = target(t);
                a["element"] = json!(element);
                a["text"] = json!(text);
                ("type", a)
            }
            Ui::Key { target: t, key } => {
                let mut a = target(t);
                a["key"] = json!(key);
                ("key", a)
            }
            Ui::Scroll {
                target: t,
                element,
                dy,
            } => {
                let mut a = target(t);
                a["element"] = json!(element);
                a["dy"] = json!(dy);
                ("scroll", a)
            }
        },
    };
    let workspace = std::fs::canonicalize(cli.workspace)?;
    let mut socket = match tokio::net::UnixStream::connect(cli.socket).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("server_unavailable: {e}");
            std::process::exit(3);
        }
    };
    wire::send(
        &mut socket,
        &Request {
            kind: kind.into(),
            workspace,
            args,
        },
    )
    .await?;
    loop {
        match wire::recv::<_, Reply>(&mut socket).await? {
            Reply::Accepted { job_id, .. } => eprintln!("job {job_id}"),
            Reply::Log { text, .. } => eprint!("{text}"),
            Reply::Status { value } => {
                println!("{}", serde_json::to_string_pretty(&value)?);
                return Ok(());
            }
            Reply::Done { result, directory } => {
                if let Some(e) = result.get("error") {
                    eprintln!("{e}");
                }
                println!("{}", directory.display());
                std::process::exit(result["exit_code"].as_i64().unwrap_or(13) as i32);
            }
            Reply::Error { error } => {
                eprintln!("{error}");
                std::process::exit(error.exit_code());
            }
        }
    }
}
