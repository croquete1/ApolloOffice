mod ai;
mod app;
mod coderabbit;
mod codex_forecast;
mod codex_usage;
mod db;
mod drop_recovery;
mod gh;
mod goog;
mod hunt_health;
mod mcp;
mod net;
mod ntf;
mod pipe;
mod prov;
mod quality;
mod resources;
mod routing;
mod sh;
mod star;
mod steward;
// This opt-in cleanup implementation is retained until its Control Room toggle ships.
#[allow(dead_code)]
mod steward_close;
mod steward_merge;
mod steward_repair;
mod sys;
mod upd;
mod vault;
mod web;
mod wk;

use app::{App, Ax};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Parser)]
#[command(
    name = "apollo-office",
    version,
    about = "AI workers that turn GitHub issues into verified bug-fix pull requests"
)]
struct A {
    #[arg(long, default_value_t = 47300)]
    port: u16,
    #[arg(long)]
    data: Option<PathBuf>,
    #[arg(long)]
    quiet: bool,
    #[command(subcommand)]
    c: Option<C>,
}

#[derive(Subcommand)]
enum C {
    #[command(about = "Print a one-time login link (10 minutes)")]
    LoginLink,
    #[command(about = "Start ApolloOffice when you log in (on|off)")]
    Autostart { on: String },
    #[command(about = "Short status")]
    Status,
    #[command(hide = true)]
    Mcp { port: u16, token: String },
}

fn prepare_data_dir(dd: &std::path::Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dd)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dd, std::fs::Permissions::from_mode(0o700));
    }
    Ok(())
}

fn open(dd: &std::path::Path) -> anyhow::Result<db::Db> {
    prepare_data_dir(dd)?;
    db::Db::open(&dd.join("office.db"))
}

// The server owns recovery and all worker lanes for this directory. A port
// lock alone cannot prevent a second instance from recovering live work.
fn lock_data_dir(dd: &std::path::Path) -> anyhow::Result<std::fs::File> {
    prepare_data_dir(dd)?;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(dd.join("office.lock"))?;
    let started = std::time::Instant::now();
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(std::fs::TryLockError::WouldBlock) => {
                // On Windows the replacement process may start just before
                // its predecessor exits. Wait briefly without stealing a lock.
                if started.elapsed() >= std::time::Duration::from_secs(5) {
                    anyhow::bail!("Apollo Office data directory is already in use: {}", dd.display());
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
        }
    }
}

fn url(d: &db::Db, port: u16) -> anyhow::Result<String> {
    Ok(format!(
        "http://127.0.0.1:{port}/auth/link?c={}",
        web::link_new(d)?
    ))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let a = A::parse();
    if let Some(C::Mcp { port, token }) = &a.c {
        mcp::bridge(*port, token.clone()).await;
        return Ok(());
    }
    let dd = app::ddir(a.data.clone());
    let me = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("apollo-office"));
    match a.c {
        Some(C::LoginLink) => {
            let d = open(&dd)?;
            let p = d.kg("port").as_u64().unwrap_or(a.port as u64) as u16;
            println!("One-time login link (valid 10 minutes):\n{}", url(&d, p)?);
            return Ok(());
        }
        Some(C::Autostart { on }) => {
            println!(
                "{}",
                sys::autostart(on == "on", &me, &dd, a.port).map_err(anyhow::Error::msg)?
            );
            return Ok(());
        }
        Some(C::Status) => {
            let d = open(&dd)?;
            println!("ApolloOffice {}  Data: {}", app::V, dd.display());
            println!(
                "Repos: {}  Machines: {}",
                d.n("select count(*) from rp", pa![]),
                d.n("select count(*) from hs", pa![])
            );
            println!("Jobs: {} in triage, {} queued, {} running, {} waiting for approval", d.n("select count(*) from jobs where st='vet'", pa![]), d.n("select count(*) from jobs where st='ready'", pa![]), d.n("select count(*) from jobs where st in ('repro','fix','ci','review','gate','submit','hunt')", pa![]), d.n("select count(*) from jobs where st='approve'", pa![]));
            println!(
                "PRs: {} open, {} merged",
                d.n("select count(*) from prs where st='open'", pa![]),
                d.n("select count(*) from prs where st='merged'", pa![])
            );
            return Ok(());
        }
        _ => {}
    }
    // Keep the same file handle alive until process exit/reexec. Never
    // remove the lock file: another process may already have opened it.
    let _instance_lock = lock_data_dir(&dd)?;
    upd::cleanup(&me);
    let d = open(&dd)?;
    d.ks("port", &serde_json::json!(a.port));
    let l = match tokio::net::TcpListener::bind(("127.0.0.1", a.port)).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!(
                "Port {} is taken ({e}). Is ApolloOffice already running? Otherwise pick --port.",
                a.port
            );
            std::process::exit(1);
        }
    };
    let ax: Ax = Arc::new(App::new(d, dd.clone(), a.port, me));
    pipe::recover(&ax)
        .map_err(|e| anyhow::anyhow!("job recovery failed; workers not started: {e}"))?;
    tokio::spawn(net::watch(ax.clone()));
    tokio::spawn(pipe::lanes(ax.clone()));
    tokio::spawn(pipe::guard(ax.clone()));
    tokio::spawn(wk::scout(ax.clone()));
    tokio::spawn(wk::vet(ax.clone()));
    tokio::spawn(wk::watch(ax.clone()));
    tokio::spawn(steward::run(ax.clone()));
    tokio::spawn(codex_usage::watch(ax.clone()));
    if !a.quiet {
        println!(
            "\n  ApolloOffice {}\n\n  Open {}\n\n  Data: {}\n  Stop with Ctrl+C\n",
            app::V,
            url(&ax.db, a.port).unwrap_or_else(|e| {
                format!("login link unavailable ({e}); retry with apollo-office login-link")
            }),
            dd.display()
        );
    }
    upd::spawn(ax.clone());
    star::spawn(ax.clone());
    let r = axum::Router::new().fallback(web::h).with_state(ax.clone());
    let q = ax.clone();
    let q2 = ax.clone();
    let sv = axum::serve(l, r).with_graceful_shutdown(async move {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = q.quit.notified() => {}
        }
    });
    tokio::select! {
        r = sv => r?,
        _ = async move {
            q2.quit.notified().await;
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        } => {}
    }
    if *ax.restart.lock().unwrap() {
        match upd::install(&ax) {
            Ok(()) => upd::relaunch(&ax.me),
            Err(e) => {
                eprintln!("update failed: {e}");
                upd::relaunch(&ax.me)
            }
        }
    }
    Ok(())
}
