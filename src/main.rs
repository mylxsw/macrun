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
    /// Target a stable client ID from status; mandatory when more than one client is online.
    #[arg(long, global = true, env = "MACRUN_CLIENT")]
    client: Option<String>,
    #[arg(long, global = true, default_value = ".")]
    workspace: PathBuf,
    #[command(subcommand)]
    command: Cmd,
}
#[derive(Subcommand)]
enum Cmd {
    /// Generate a single-use invitation, valid for ten minutes.
    Invite {
        #[arg(long)]
        data: PathBuf,
        #[arg(long)]
        server: String,
    },
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
        #[arg(long, required_unless_present = "connections")]
        server: Option<std::net::SocketAddr>,
        #[arg(long, required_unless_present = "connections")]
        cert: Option<PathBuf>,
        #[arg(long, required_unless_present = "connections")]
        token_file: Option<PathBuf>,
        /// JSON array of named server profiles. Credentials stay in files or the inherited pipe.
        #[arg(long, conflicts_with_all=["server","cert","token_file","config"])]
        connections: Option<PathBuf>,
        /// Read credentials from the inherited pipe, never from process arguments.
        #[arg(long)]
        token_stdin: bool,
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
        workspace_root: Option<String>,
        #[arg(long)]
        generation: Option<String>,
        #[arg(long, default_value_t = 0)]
        wait_ms: u64,
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
    /// Synchronize once; --watch observes filesystem changes. Does not run commands.
    Sync {
        /// Bypass metadata hash caches on both ends.
        #[arg(long)]
        strict: bool,
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
        /// Reuse this UUID to resume an interrupted binary upload.
        #[arg(long)]
        transfer_id: Option<String>,
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
        Cmd::Invite { data, server } => {
            println!("{}", macrun::pairing::create(&data, server)?);
            return Ok(());
        }
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
            connections,
            token_stdin,
            data,
            config,
            control_socket,
            parent_pipe,
        } => {
            let injected = if token_stdin {
                use std::io::Read;
                let mut header = [0u8; 4];
                std::io::stdin().read_exact(&mut header)?;
                let n = u32::from_be_bytes(header) as usize;
                anyhow::ensure!(n <= 65536, "credential frame too large");
                let mut b = vec![0; n];
                std::io::stdin().read_exact(&mut b)?;
                Some(String::from_utf8(b)?)
            } else {
                None
            };
            if let Some(path) = connections {
                let profiles: Vec<macrun::worker::Profile> =
                    serde_json::from_slice(&std::fs::read(path)?)?;
                let tokens = if let Some(frame) = injected {
                    serde_json::from_str(&frame)?
                } else {
                    Default::default()
                };
                return macrun::worker::worker_profiles(
                    profiles,
                    control_socket,
                    tokens,
                    parent_pipe,
                )
                .await;
            }
            return macrun::worker::worker_managed(
                macrun::worker::Options {
                    server: server.expect("server required").to_string(),
                    cert: cert.expect("cert required"),
                    token_file: token_file.expect("token file required"),
                    data,
                    config,
                },
                control_socket,
                injected,
                parent_pipe,
            )
            .await;
        }
        Cmd::Mcp => return frontend::mcp_for(cli.socket, root, cli.client).await,
        Cmd::Status => ("status".into(), json!({})),
        Cmd::Call { operation, args } => (operation, serde_json::from_str(&args)?),
        Cmd::Exec {
            cwd,
            workspace_root,
            generation,
            wait_ms,
            request_id,
            timeout,
            command,
        } => (
            "exec.start".into(),
            json!({"cwd":cwd,"workspace_root":workspace_root,"generation":generation,"wait_ms":wait_ms,"command":command,"timeout_seconds":timeout,"request_id":request_id.unwrap_or_else(id)}),
        ),
        Cmd::Task { task_id, offset } => (
            "task.get".into(),
            json!({"task_id":task_id,"offset":offset}),
        ),
        Cmd::Cancel { task_id } => ("task.cancel".into(), json!({"task_id":task_id})),
        Cmd::Sync {
            watch,
            interval_ms,
            strict,
        } => {
            if interval_ms == 0 {
                anyhow::bail!("interval must be positive");
            }
            let mut watcher = if watch {
                match macrun::watch::source(&root) {
                    Ok(w) => Some(w),
                    Err(e) => {
                        eprintln!("watch unavailable; using polling: {e}");
                        None
                    }
                }
            } else {
                None
            };
            let mut verified = std::time::Instant::now();
            loop {
                if let Some((_, rx)) = &mut watcher {
                    rx.borrow_and_update();
                }
                let verify = strict || verified.elapsed() >= Duration::from_secs(600);
                if verify {
                    verified = std::time::Instant::now();
                }
                let operation = async {
                    let accepted = frontend::request_for(
                        &cli.socket,
                        &root,
                        "sync",
                        json!({"detach":true,"strict":verify}),
                        cli.client.as_deref(),
                    )
                    .await?;
                    let jid = accepted["job_id"].clone();
                    loop {
                        let v = frontend::request_for(
                            &cli.socket,
                            &root,
                            "sync.get",
                            json!({"job_id":jid}),
                            cli.client.as_deref(),
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
                let succeeded = r.is_ok();
                if let Err(e) = r {
                    eprintln!("sync retry: {e}");
                }
                if succeeded && let Some((_, rx)) = &mut watcher {
                    let remaining = Duration::from_secs(600).saturating_sub(verified.elapsed());
                    tokio::select! {_=rx.changed()=>{},_=tokio::time::sleep(remaining)=>{},_=tokio::signal::ctrl_c()=>return Ok(())}
                }
                tokio::select! {_=tokio::time::sleep(Duration::from_millis(interval_ms))=>{},_=tokio::signal::ctrl_c()=>return Ok(())}
            }
        }
        Cmd::Download { remote, local } => {
            if macrun::transfer::capable(&cli.socket, &root, cli.client.as_deref()).await? {
                let temp = local.with_extension(format!("{}.part", id()));
                let mut file = tokio::fs::File::create(&temp).await?;
                let result = macrun::transfer::download(
                    &cli.socket,
                    &root,
                    "file.download",
                    json!({"path":remote}),
                    cli.client.as_deref(),
                    &mut file,
                )
                .await;
                if let Err(error) = result {
                    drop(file);
                    let _ = tokio::fs::remove_file(&temp).await;
                    return Err(error);
                }
                file.sync_all().await?;
                drop(file);
                tokio::fs::rename(temp, &local).await?;
                println!("{}", local.display());
                return Ok(());
            }
            let temp = local.with_extension(format!("{}.part", id()));
            let mut f = tokio::fs::File::create(&temp).await?;
            let mut offset = 0;
            let mut version = None;
            loop {
                let v = frontend::request_for(
                    &cli.socket,
                    &root,
                    "file.read",
                    json!({"path":remote,"offset":offset}),
                    cli.client.as_deref(),
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
        Cmd::Upload {
            local,
            remote,
            transfer_id,
        } => {
            if macrun::transfer::capable(&cli.socket, &root, cli.client.as_deref()).await? {
                let transfer_id = transfer_id.unwrap_or_else(id);
                let upload = macrun::transfer::upload(
                    &cli.socket,
                    &root,
                    &local,
                    &remote,
                    cli.client.as_deref(),
                    &transfer_id,
                )
                .await;
                if let Err(error) = upload {
                    anyhow::bail!("{error}; resume with --transfer-id {transfer_id}");
                }
                println!("{remote}");
                return Ok(());
            }
            let temp = format!("{remote}.macrun-upload-{}", id());
            let mut f = tokio::fs::File::open(local).await?;
            let mut offset = 0;
            let mut b = vec![0; macrun::files::CHUNK];
            loop {
                let n = f.read(&mut b).await?;
                frontend::request_for(&cli.socket,&root,"file.write",json!({"path":temp,"offset":offset,"truncate":offset==0,"data":STANDARD.encode(&b[..n])}),cli.client.as_deref()).await?;
                offset += n as u64;
                if n == 0 {
                    break;
                }
            }
            frontend::request_for(
                &cli.socket,
                &root,
                "file.move",
                json!({"path":temp,"destination":remote}),
                cli.client.as_deref(),
            )
            .await?;
            println!("{remote}");
            return Ok(());
        }
    };
    let result =
        frontend::request_for(&cli.socket, &root, &kind, args, cli.client.as_deref()).await?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}
