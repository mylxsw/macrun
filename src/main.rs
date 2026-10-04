use base64::{Engine, engine::general_purpose::STANDARD};
use clap::{Parser, Subcommand};
use macrun::{frontend, model::id, wire};
use serde_json::json;
use std::{path::PathBuf, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[derive(Parser)]
#[command(
    version,
    about = "Generic remote commands, files, directory sync and local MCP tools"
)]
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
        #[arg(long)]
        config: Option<PathBuf>,
        /// Private local control socket for the desktop application.
        #[arg(long)]
        control_socket: Option<PathBuf>,
        /// Stop the worker and its tasks when the desktop parent closes stdin.
        #[arg(long)]
        parent_pipe: bool,
    },
    /// Expose generic tools to Claude Code using stdio MCP.
    Mcp,
    Status,
    /// Call any generic operation with JSON arguments (e.g. file.read, mcp.tools).
    Call {
        operation: String,
        #[arg(long, default_value = "{}")]
        args: String,
    },
    Exec {
        #[arg(long)]
        cwd: String,
        #[arg(long)]
        request_id: Option<String>,
        #[arg(long, default_value_t = 3600)]
        timeout: u64,
        command: String,
    },
    Task {
        task_id: String,
        #[arg(long, default_value_t = 0)]
        offset: u64,
    },
    Cancel {
        task_id: String,
    },
    /// Synchronize once; --watch continuously polls for changes. Does not run commands.
    Sync {
        #[arg(long)]
        watch: bool,
        #[arg(long, default_value_t = 1000)]
        interval_ms: u64,
    },
    Download {
        remote: String,
        local: PathBuf,
    },
    Upload {
        local: PathBuf,
        remote: String,
    },
}
#[tokio::main]
async fn main() {
    if let Err(e) = entry().await {
        eprintln!("{e:#}");
        std::process::exit(1);
    }
}
async fn entry() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let root = std::fs::canonicalize(&cli.workspace)?;
    let (kind, args) = match cli.command {
        Cmd::Init { data } => {
            wire::init_tls(&data)?;
            println!("Created identity: {}", data.display());
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
            config,
            control_socket,
            parent_pipe,
        } => {
            return macrun::worker::worker_managed(
                macrun::worker::Options {
                    server,
                    cert,
                    token_file,
                    data,
                    config,
                },
                control_socket,
                None,
                parent_pipe,
            )
            .await;
        }
        Cmd::Mcp => return frontend::mcp(cli.socket, root).await,
        Cmd::Status => ("status".into(), json!({})),
        Cmd::Call { operation, args } => (operation, serde_json::from_str(&args)?),
        Cmd::Exec {
            cwd,
            request_id,
            timeout,
            command,
        } => (
            "exec.start".into(),
            json!({"cwd":cwd,"command":command,"timeout_seconds":timeout,"request_id":request_id.unwrap_or_else(id)}),
        ),
        Cmd::Task { task_id, offset } => (
            "task.get".into(),
            json!({"task_id":task_id,"offset":offset}),
        ),
        Cmd::Cancel { task_id } => ("task.cancel".into(), json!({"task_id":task_id})),
        Cmd::Sync { watch, interval_ms } => {
            if interval_ms == 0 {
                anyhow::bail!("interval must be positive");
            }
            loop {
                let operation = async {
                    let accepted =
                        frontend::request(&cli.socket, &root, "sync", json!({"detach":true}))
                            .await?;
                    let jid = accepted["job_id"].clone();
                    loop {
                        let v = frontend::request(
                            &cli.socket,
                            &root,
                            "sync.get",
                            json!({"job_id":jid}),
                        )
                        .await?;
                        if v.get("ended_at").is_some() {
                            if v["status"] != "succeeded" {
                                anyhow::bail!("{v}");
                            }
                            println!("{}", serde_json::to_string(&v)?);
                            return Ok::<_, anyhow::Error>(());
                        }
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                };
                let r = tokio::select! {r=operation=>r,_=tokio::signal::ctrl_c()=>return Ok(())};
                if !watch {
                    return r;
                }
                if let Err(e) = r {
                    eprintln!("sync retry: {e}");
                }
                tokio::select! {_=tokio::time::sleep(Duration::from_millis(interval_ms))=>{},_=tokio::signal::ctrl_c()=>return Ok(())}
            }
        }
        Cmd::Download { remote, local } => {
            let temp = local.with_extension(format!("{}.part", id()));
            let mut f = tokio::fs::File::create(&temp).await?;
            let mut offset = 0;
            let mut version = None;
            loop {
                let v = frontend::request(
                    &cli.socket,
                    &root,
                    "file.read",
                    json!({"path":remote,"offset":offset}),
                )
                .await?;
                if version.as_ref().is_some_and(|old| old != &v["version"]) {
                    anyhow::bail!(
                        "remote file changed; partial download retained at {}",
                        temp.display()
                    );
                }
                version = Some(v["version"].clone());
                f.write_all(&STANDARD.decode(v["data"].as_str().unwrap_or(""))?)
                    .await?;
                offset = v["next_offset"].as_u64().unwrap();
                if v["eof"] == true {
                    break;
                }
            }
            f.sync_all().await?;
            drop(f);
            tokio::fs::rename(&temp, &local).await?;
            println!("{}", local.display());
            return Ok(());
        }
        Cmd::Upload { local, remote } => {
            let temp = format!("{remote}.macrun-upload-{}", id());
            let mut f = tokio::fs::File::open(local).await?;
            let mut offset = 0;
            let mut b = vec![0; macrun::files::CHUNK];
            loop {
                let n = f.read(&mut b).await?;
                frontend::request(&cli.socket,&root,"file.write",json!({"path":temp,"offset":offset,"truncate":offset==0,"data":STANDARD.encode(&b[..n])})).await?;
                offset += n as u64;
                if n == 0 {
                    break;
                }
            }
            frontend::request(
                &cli.socket,
                &root,
                "file.move",
                json!({"path":temp,"destination":remote}),
            )
            .await?;
            println!("{remote}");
            return Ok(());
        }
    };
    let result = frontend::request(&cli.socket, &root, &kind, args).await?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}
